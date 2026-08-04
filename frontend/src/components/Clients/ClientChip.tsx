'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Briefcase, Check, ChevronDown, Plus, X } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Client, ClientSuggestion, ClientWithCounts } from '@/types/clients';

interface ClientChipProps {
  meetingId: string;
  /** Called whenever the meeting's client tag changes (accept, change, clear). */
  onClientChange?: (client: Client | null) => void;
}

/**
 * Client selector chip for the meeting-details header. Shows the tagged
 * client, or a suggestion banner (accept / change / dismiss) when the meeting
 * is untagged and a client matches by attendee domain or title. Suggestions
 * are never applied silently.
 */
export function ClientChip({ meetingId, onClientChange }: ClientChipProps) {
  const [client, setClient] = useState<Client | null>(null);
  const [suggestion, setSuggestion] = useState<ClientSuggestion | null>(null);
  const [clients, setClients] = useState<ClientWithCounts[]>([]);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [newName, setNewName] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setClient(null);
    setSuggestion(null);
    void (async () => {
      try {
        const tagged = await invoke<Client | null>('meeting_get_client', { meetingId });
        if (cancelled) return;
        setClient(tagged);
        if (!tagged) {
          const suggested = await invoke<ClientSuggestion | null>('meeting_suggest_client', {
            meetingId,
          });
          if (!cancelled) setSuggestion(suggested);
        }
      } catch (error) {
        console.error('Failed to load meeting client:', error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  const loadClients = useCallback(async () => {
    try {
      setClients(await invoke<ClientWithCounts[]>('clients_list'));
    } catch (error) {
      console.error('Failed to load clients:', error);
    }
  }, []);

  useEffect(() => {
    if (pickerOpen) void loadClients();
  }, [pickerOpen, loadClients]);

  const applyClient = useCallback(
    async (clientId: string | null) => {
      setBusy(true);
      try {
        const updated = await invoke<Client | null>('meeting_set_client', {
          meetingId,
          clientId,
        });
        setClient(updated);
        setSuggestion(null);
        setPickerOpen(false);
        onClientChange?.(updated);
      } catch (error) {
        console.error('Failed to set meeting client:', error);
        toast.error('Failed to tag meeting', {
          description: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setBusy(false);
      }
    },
    [meetingId, onClientChange],
  );

  const handleCreateAndTag = useCallback(async () => {
    const name = newName.trim();
    if (!name) return;
    setBusy(true);
    try {
      const created = await invoke<Client>('client_create', {
        name,
        domain: null,
        notes: null,
      });
      setNewName('');
      await applyClient(created.id);
    } catch (error) {
      console.error('Failed to create client:', error);
      toast.error('Failed to create client', {
        description: error instanceof Error ? error.message : String(error),
      });
      setBusy(false);
    }
  }, [newName, applyClient]);

  const picker = (
    <PopoverContent align="start" className="w-72 p-2 space-y-1">
      <div className="text-xs font-medium text-muted-ink px-1 pb-1">Tag this meeting</div>
      <div className="max-h-56 overflow-y-auto custom-scrollbar space-y-0.5">
        {clients.map(candidate => (
          <button
            key={candidate.id}
            disabled={busy}
            onClick={() => void applyClient(candidate.id)}
            className="w-full flex items-center gap-2 px-2 py-1.5 text-sm rounded-md text-left text-ink hover:bg-active"
          >
            <span className="flex-1 truncate">{candidate.name}</span>
            {client?.id === candidate.id && <Check className="w-3.5 h-3.5 text-muted-ink" />}
          </button>
        ))}
        {clients.length === 0 && (
          <div className="px-2 py-1.5 text-sm text-faint">No clients yet. Create one below.</div>
        )}
      </div>
      <div className="flex items-center gap-1 pt-1 border-t border-edge">
        <input
          value={newName}
          onChange={event => setNewName(event.target.value)}
          onKeyDown={event => {
            if (event.key === 'Enter') void handleCreateAndTag();
          }}
          placeholder="New client name…"
          className="flex-1 min-w-0 rounded-md border border-edge bg-surface px-2 py-1 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
        />
        <button
          disabled={!newName.trim() || busy}
          onClick={() => void handleCreateAndTag()}
          className="p-1.5 rounded-md text-muted-ink hover:bg-active disabled:opacity-40"
          title="Create client and tag meeting"
          aria-label="Create client and tag meeting"
        >
          <Plus className="w-4 h-4" />
        </button>
      </div>
      {client && (
        <button
          disabled={busy}
          onClick={() => void applyClient(null)}
          className="w-full px-2 py-1.5 text-xs text-left text-faint hover:text-red-600 rounded-md hover:bg-active"
        >
          Remove client tag
        </button>
      )}
    </PopoverContent>
  );

  return (
    <div className="flex items-center gap-2 min-w-0">
      <Popover open={pickerOpen} onOpenChange={setPickerOpen}>
        <PopoverTrigger asChild>
          <button
            className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md border text-xs transition-colors ${
              client
                ? 'border-edge bg-wash text-ink font-medium hover:bg-active'
                : 'border-dashed border-edge text-faint hover:text-muted-ink hover:bg-wash'
            }`}
            title={client ? `Client: ${client.name}` : 'Tag this meeting with a client'}
          >
            <Briefcase className="w-3.5 h-3.5" />
            <span className="truncate max-w-[160px]">{client ? client.name : 'No client'}</span>
            <ChevronDown className="w-3 h-3" />
          </button>
        </PopoverTrigger>
        {picker}
      </Popover>

      {!client && suggestion && (
        <div className="flex items-center gap-1.5 min-w-0 text-xs text-muted-ink bg-wash border border-edge rounded-md px-2 py-1">
          <span className="truncate" title={suggestion.reason}>
            Looks like <span className="font-medium text-ink">{suggestion.client_name}</span>
          </span>
          <button
            disabled={busy}
            onClick={() => void applyClient(suggestion.client_id)}
            className="text-blue-600 hover:underline font-medium"
          >
            Tag
          </button>
          <button
            disabled={busy}
            onClick={() => setPickerOpen(true)}
            className="text-muted-ink hover:underline"
          >
            Change
          </button>
          <button
            onClick={() => setSuggestion(null)}
            className="text-faint hover:text-muted-ink"
            title="Dismiss suggestion"
            aria-label="Dismiss suggestion"
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      )}
    </div>
  );
}
