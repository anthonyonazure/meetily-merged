'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { ChevronDown, ChevronUp, Loader2, MessageSquare, SendHorizontal, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { MarkdownLite } from '@/components/shared/MarkdownLite';
import { useConfig } from '@/contexts/ConfigContext';
import { ChatMessageRecord, ChatResponsePayload, ChatSendResult } from '@/types/chat';
import { Client } from '@/types/clients';

const POLL_INTERVAL_MS = 4000;

type ChatScope = 'meeting' | 'all' | 'client';

interface ChatPanelProps {
  /** Meeting the panel is mounted on (meeting-details). */
  meetingId?: string;
  /** Locks the panel to one client's thread (Clients page). */
  fixedClient?: { id: string; name: string };
}

export function ChatPanel({ meetingId, fixedClient }: ChatPanelProps) {
  const { modelConfig } = useConfig();
  const [expanded, setExpanded] = useState(false);
  const [scope, setScope] = useState<ChatScope>(fixedClient ? 'client' : 'meeting');
  const [meetingClient, setMeetingClient] = useState<Client | null>(null);
  const [messages, setMessages] = useState<ChatMessageRecord[]>([]);
  const [input, setInput] = useState('');
  const [waiting, setWaiting] = useState(false);
  const listRef = useRef<HTMLDivElement | null>(null);

  const activeClient = fixedClient ?? (meetingClient ? { id: meetingClient.id, name: meetingClient.name } : null);

  // The ids sent to the backend for the current scope.
  const scopeMeetingId = scope === 'meeting' && meetingId ? meetingId : null;
  const scopeClientId = scope === 'client' && activeClient ? activeClient.id : null;
  // One string key so async handlers can detect scope switches.
  const scopeKey = scopeClientId
    ? `client:${scopeClientId}`
    : scopeMeetingId
      ? `meeting:${scopeMeetingId}`
      : 'all';
  const scopeKeyRef = useRef(scopeKey);
  scopeKeyRef.current = scopeKey;

  // On meeting-details, the Client scope is offered when the meeting is tagged.
  useEffect(() => {
    if (fixedClient || !meetingId) return;
    let cancelled = false;
    setMeetingClient(null);
    void (async () => {
      try {
        const tagged = await invoke<Client | null>('meeting_get_client', { meetingId });
        if (!cancelled) setMeetingClient(tagged);
      } catch (error) {
        console.error('Failed to load meeting client for chat:', error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [meetingId, fixedClient]);

  // Leave the client scope if the meeting changes to one without that client.
  useEffect(() => {
    if (!fixedClient && scope === 'client' && !meetingClient) setScope('meeting');
  }, [fixedClient, scope, meetingClient]);

  const refresh = useCallback(async () => {
    const requestedKey = scopeKeyRef.current;
    const [kind, id] = requestedKey === 'all' ? ['all', null] : (requestedKey.split(':') as [string, string]);
    try {
      const history = await invoke<ChatMessageRecord[]>('chat_history', {
        meetingId: kind === 'meeting' ? id : null,
        clientId: kind === 'client' ? id : null,
      });
      // Ignore stale responses after a meeting or scope switch.
      if (scopeKeyRef.current !== requestedKey) return;
      setMessages(history);
      // An assistant message after the last user message means nothing is pending.
      const last = history[history.length - 1];
      if (!last || last.role === 'assistant') setWaiting(false);
    } catch (error) {
      console.error('Failed to load chat history:', error);
    }
  }, []);

  // Load on mount and whenever the scope key changes.
  useEffect(() => {
    setMessages([]);
    setWaiting(false);
    void refresh();
  }, [scopeKey, refresh]);

  // Event push: refresh when a response for our scope arrives.
  useEffect(() => {
    const unlistenPromise = listen<ChatResponsePayload>('chat-response', event => {
      const payloadKey = event.payload.client_id
        ? `client:${event.payload.client_id}`
        : event.payload.meeting_id
          ? `meeting:${event.payload.meeting_id}`
          : 'all';
      if (payloadKey === scopeKeyRef.current) void refresh();
    });
    return () => {
      void unlistenPromise.then(unlisten => unlisten());
    };
  }, [refresh]);

  // Poll fallback while a response is pending (events can be missed across
  // navigation).
  useEffect(() => {
    if (!waiting) return;
    const interval = setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [waiting, refresh]);

  // Keep the newest message in view.
  useEffect(() => {
    const list = listRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [messages, waiting, expanded]);

  const handleSend = useCallback(async () => {
    const question = input.trim();
    if (!question || waiting) return;
    if (!modelConfig.provider || !modelConfig.model) {
      toast.error('Configure a summary model first', {
        description: 'Chat uses the same AI provider as summaries.',
      });
      return;
    }
    setInput('');
    setWaiting(true);
    // Optimistic echo of the user message; refresh replaces it with the stored row.
    setMessages(previous => [
      ...previous,
      {
        id: `pending-${Date.now()}`,
        meeting_id: scopeMeetingId,
        client_id: scopeClientId,
        role: 'user',
        content: question,
        created_at: new Date().toISOString(),
      },
    ]);
    try {
      await invoke<ChatSendResult>('chat_send', {
        meetingId: scopeMeetingId,
        clientId: scopeClientId,
        message: question,
        modelProvider: modelConfig.provider,
        modelName: modelConfig.model,
      });
    } catch (error) {
      console.error('Failed to send chat message:', error);
      setWaiting(false);
      toast.error('Failed to send message', {
        description: error instanceof Error ? error.message : String(error),
      });
      void refresh();
    }
  }, [input, waiting, modelConfig.provider, modelConfig.model, scopeMeetingId, scopeClientId, refresh]);

  const handleClear = useCallback(async () => {
    try {
      await invoke<number>('chat_clear', {
        meetingId: scopeMeetingId,
        clientId: scopeClientId,
      });
      setMessages([]);
      setWaiting(false);
    } catch (error) {
      console.error('Failed to clear chat history:', error);
      toast.error('Failed to clear chat history');
    }
  }, [scopeMeetingId, scopeClientId]);

  const emptyHint =
    scope === 'meeting'
      ? 'Ask a question about this meeting. Answers come only from its transcript and summary.'
      : scope === 'client'
        ? `Ask about ${activeClient?.name ?? 'this client'}. Answers come from their recent meetings and memory facts.`
        : 'Ask a question across your recent meetings. Answers come from their titles and summaries.';

  const placeholder =
    scope === 'meeting'
      ? 'Ask about this meeting…'
      : scope === 'client'
        ? `Ask about ${activeClient?.name ?? 'this client'}…`
        : 'Ask across your meetings…';

  return (
    <div className="border-t border-edge bg-surface flex-shrink-0 flex flex-col max-h-[45%]">
      {/* Header bar */}
      <button
        onClick={() => setExpanded(previous => !previous)}
        className="flex items-center gap-2 px-4 py-2.5 text-sm font-medium text-muted-ink hover:bg-wash transition-colors w-full text-left"
      >
        <MessageSquare className="w-4 h-4 text-muted-ink" />
        <span>Chat</span>
        {waiting && (
          <span className="flex items-center gap-1 text-xs text-blue-600">
            <Loader2 className="w-3 h-3 animate-spin" />
            Thinking
          </span>
        )}
        <span className="ml-auto text-faint">
          {expanded ? <ChevronDown className="w-4 h-4" /> : <ChevronUp className="w-4 h-4" />}
        </span>
      </button>

      {expanded && (
        <div className="flex flex-col min-h-0">
          {/* Scope switch and clear */}
          <div className="flex items-center gap-2 px-4 pb-2">
            {!fixedClient && (
              <div className="flex rounded-md border border-edge overflow-hidden text-xs">
                <button
                  onClick={() => setScope('meeting')}
                  className={`px-2.5 py-1 ${scope === 'meeting' ? 'bg-wash text-ink font-medium' : 'text-muted-ink hover:bg-wash'}`}
                >
                  This meeting
                </button>
                {meetingClient && (
                  <button
                    onClick={() => setScope('client')}
                    className={`px-2.5 py-1 border-l border-edge ${scope === 'client' ? 'bg-wash text-ink font-medium' : 'text-muted-ink hover:bg-wash'}`}
                    title={`Ask across everything tagged ${meetingClient.name}`}
                  >
                    {meetingClient.name}
                  </button>
                )}
                <button
                  onClick={() => setScope('all')}
                  className={`px-2.5 py-1 border-l border-edge ${scope === 'all' ? 'bg-wash text-ink font-medium' : 'text-muted-ink hover:bg-wash'}`}
                >
                  All meetings
                </button>
              </div>
            )}
            {fixedClient && (
              <span className="text-xs text-muted-ink">
                Asking about <span className="font-medium text-ink">{fixedClient.name}</span>
              </span>
            )}
            {messages.length > 0 && (
              <Button
                variant="ghost"
                size="sm"
                className="ml-auto text-faint"
                title="Clear chat history"
                aria-label="Clear chat history"
                onClick={() => void handleClear()}
              >
                <Trash2 className="w-3.5 h-3.5" />
              </Button>
            )}
          </div>

          {/* Message list */}
          <div ref={listRef} className="overflow-y-auto px-4 space-y-3 min-h-[80px] max-h-[240px]">
            {messages.length === 0 && !waiting && (
              <div className="text-sm text-faint py-2">{emptyHint}</div>
            )}
            {messages.map(message => (
              <div key={message.id} className={message.role === 'user' ? 'text-right' : 'text-left'}>
                <div
                  className={
                    message.role === 'user'
                      ? 'inline-block max-w-[85%] rounded-lg bg-blue-50 px-3 py-2 text-sm text-ink text-left'
                      : 'inline-block max-w-[85%] rounded-lg bg-app px-3 py-2 text-sm text-ink text-left'
                  }
                >
                  {message.role === 'assistant' ? (
                    <MarkdownLite markdown={message.content} />
                  ) : (
                    <span className="whitespace-pre-wrap">{message.content}</span>
                  )}
                </div>
              </div>
            ))}
            {waiting && (
              <div className="flex items-center gap-2 text-xs text-faint pb-1">
                <Loader2 className="w-3 h-3 animate-spin" />
                Waiting for the model…
              </div>
            )}
          </div>

          {/* Input row */}
          <div className="flex items-end gap-2 p-3">
            <textarea
              value={input}
              onChange={event => setInput(event.target.value)}
              onKeyDown={event => {
                if (event.key === 'Enter' && !event.shiftKey) {
                  event.preventDefault();
                  void handleSend();
                }
              }}
              rows={1}
              placeholder={placeholder}
              className="flex-1 resize-none rounded-md border border-edge px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
            />
            <Button
              variant="outline"
              size="sm"
              disabled={!input.trim() || waiting}
              onClick={() => void handleSend()}
              title="Send"
              aria-label="Send"
            >
              <SendHorizontal className="w-4 h-4" />
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
