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

const POLL_INTERVAL_MS = 4000;

type ChatScope = 'meeting' | 'all';

interface ChatPanelProps {
  meetingId: string;
}

export function ChatPanel({ meetingId }: ChatPanelProps) {
  const { modelConfig } = useConfig();
  const [expanded, setExpanded] = useState(false);
  const [scope, setScope] = useState<ChatScope>('meeting');
  const [messages, setMessages] = useState<ChatMessageRecord[]>([]);
  const [input, setInput] = useState('');
  const [waiting, setWaiting] = useState(false);
  const listRef = useRef<HTMLDivElement | null>(null);
  const meetingIdRef = useRef(meetingId);
  meetingIdRef.current = meetingId;
  const scopeRef = useRef(scope);
  scopeRef.current = scope;

  // The backend scope id: the meeting id, or null for the all-meetings thread.
  const scopeMeetingId = scope === 'meeting' ? meetingId : null;

  const refresh = useCallback(async () => {
    const requestedMeetingId = meetingIdRef.current;
    const requestedScope = scopeRef.current;
    try {
      const history = await invoke<ChatMessageRecord[]>('chat_history', {
        meetingId: requestedScope === 'meeting' ? requestedMeetingId : null,
      });
      // Ignore stale responses after a meeting or scope switch.
      if (meetingIdRef.current !== requestedMeetingId || scopeRef.current !== requestedScope) return;
      setMessages(history);
      // An assistant message after the last user message means nothing is pending.
      const last = history[history.length - 1];
      if (!last || last.role === 'assistant') setWaiting(false);
    } catch (error) {
      console.error('Failed to load chat history:', error);
    }
  }, []);

  // Load on mount and whenever the meeting or scope changes.
  useEffect(() => {
    setMessages([]);
    setWaiting(false);
    void refresh();
  }, [meetingId, scope, refresh]);

  // Event push: refresh when a response for our scope arrives.
  useEffect(() => {
    const unlistenPromise = listen<ChatResponsePayload>('chat-response', event => {
      const payloadScope = event.payload.meeting_id;
      const currentScope = scopeRef.current === 'meeting' ? meetingIdRef.current : null;
      if (payloadScope === currentScope) void refresh();
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
        role: 'user',
        content: question,
        created_at: new Date().toISOString(),
      },
    ]);
    try {
      await invoke<ChatSendResult>('chat_send', {
        meetingId: scopeMeetingId,
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
  }, [input, waiting, modelConfig.provider, modelConfig.model, scopeMeetingId, refresh]);

  const handleClear = useCallback(async () => {
    try {
      await invoke<number>('chat_clear', { meetingId: scopeMeetingId });
      setMessages([]);
      setWaiting(false);
    } catch (error) {
      console.error('Failed to clear chat history:', error);
      toast.error('Failed to clear chat history');
    }
  }, [scopeMeetingId]);

  return (
    <div className="border-t border-gray-200 bg-white flex-shrink-0 flex flex-col max-h-[45%]">
      {/* Header bar */}
      <button
        onClick={() => setExpanded(previous => !previous)}
        className="flex items-center gap-2 px-4 py-2.5 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors w-full text-left"
      >
        <MessageSquare className="w-4 h-4 text-gray-500" />
        <span>Chat</span>
        {waiting && (
          <span className="flex items-center gap-1 text-xs text-blue-600">
            <Loader2 className="w-3 h-3 animate-spin" />
            Thinking
          </span>
        )}
        <span className="ml-auto text-gray-400">
          {expanded ? <ChevronDown className="w-4 h-4" /> : <ChevronUp className="w-4 h-4" />}
        </span>
      </button>

      {expanded && (
        <div className="flex flex-col min-h-0">
          {/* Scope switch and clear */}
          <div className="flex items-center gap-2 px-4 pb-2">
            <div className="flex rounded-md border border-gray-200 overflow-hidden text-xs">
              <button
                onClick={() => setScope('meeting')}
                className={`px-2.5 py-1 ${scope === 'meeting' ? 'bg-gray-100 text-gray-800 font-medium' : 'text-gray-500 hover:bg-gray-50'}`}
              >
                This meeting
              </button>
              <button
                onClick={() => setScope('all')}
                className={`px-2.5 py-1 border-l border-gray-200 ${scope === 'all' ? 'bg-gray-100 text-gray-800 font-medium' : 'text-gray-500 hover:bg-gray-50'}`}
              >
                All meetings
              </button>
            </div>
            {messages.length > 0 && (
              <Button
                variant="ghost"
                size="sm"
                className="ml-auto text-gray-400"
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
              <div className="text-sm text-gray-400 py-2">
                {scope === 'meeting'
                  ? 'Ask a question about this meeting. Answers come only from its transcript and summary.'
                  : 'Ask a question across your recent meetings. Answers come from their titles and summaries.'}
              </div>
            )}
            {messages.map(message => (
              <div key={message.id} className={message.role === 'user' ? 'text-right' : 'text-left'}>
                <div
                  className={
                    message.role === 'user'
                      ? 'inline-block max-w-[85%] rounded-lg bg-blue-50 px-3 py-2 text-sm text-gray-800 text-left'
                      : 'inline-block max-w-[85%] rounded-lg bg-gray-50 px-3 py-2 text-sm text-gray-800 text-left'
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
              <div className="flex items-center gap-2 text-xs text-gray-400 pb-1">
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
              placeholder={scope === 'meeting' ? 'Ask about this meeting…' : 'Ask across your meetings…'}
              className="flex-1 resize-none rounded-md border border-gray-200 px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
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
