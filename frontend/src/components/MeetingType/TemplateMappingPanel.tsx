'use client';

/**
 * Settings → Summary: which summary template each kind of meeting should use, at
 * the workspace level and per client.
 *
 * Left unset, nothing changes: the template picked in the generator is used, as
 * before. A mapping only fires when the detector is confident enough, and the chip
 * on meeting details always says which template ran and why.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Tag } from 'lucide-react';
import type { ClientWithCounts } from '@/types/clients';
import type { MeetingTypeMappings, MeetingTypeValue } from '@/types/meetingType';

const NO_MAPPING = '';

export function TemplateMappingPanel() {
  const [data, setData] = useState<MeetingTypeMappings | null>(null);
  const [templates, setTemplates] = useState<string[]>([]);
  const [clients, setClients] = useState<ClientWithCounts[]>([]);
  const [scope, setScope] = useState<string>('');
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setData(await invoke<MeetingTypeMappings>('meeting_type_mappings_get'));
    } catch (error) {
      console.error('Failed to load the template mappings:', error);
    }
  }, []);

  useEffect(() => {
    void load();
    void (async () => {
      try {
        const list = await invoke<Array<{ id: string }>>('api_list_templates');
        setTemplates(list.map(entry => entry.id));
      } catch (error) {
        console.error('Failed to load templates:', error);
      }
      try {
        setClients(await invoke<ClientWithCounts[]>('clients_list'));
      } catch (error) {
        console.error('Failed to load clients:', error);
      }
    })();
  }, [load]);

  const currentFor = (type: MeetingTypeValue): string => {
    if (!data) return NO_MAPPING;
    const wanted = scope === '' ? null : scope;
    return (
      data.mappings.find(m => m.meeting_type === type && m.client_id === wanted)?.template_id ??
      NO_MAPPING
    );
  };

  const setMapping = async (type: MeetingTypeValue, templateId: string) => {
    setBusy(true);
    try {
      setData(
        await invoke<MeetingTypeMappings>('meeting_type_mappings_set', {
          input: {
            meeting_type: type,
            client_id: scope === '' ? null : scope,
            template_id: templateId,
          },
        }),
      );
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  if (!data) return null;

  return (
    <div className="space-y-4 max-w-2xl">
      <div className="flex items-start gap-3">
        <Tag className="w-5 h-5 text-muted-ink mt-0.5" />
        <div>
          <h3 className="text-base font-display font-semibold text-ink">
            Template by meeting type
          </h3>
          <p className="text-sm text-muted-ink mt-1">
            After a summary is generated, the meeting is classified from its own transcript.
            If a type is mapped here and the detector is at least{' '}
            {Math.round(data.min_confidence * 100)}% confident, the next summary uses that
            template automatically. The chip on the meeting always shows which template ran.
          </p>
        </div>
      </div>

      <label className="block text-sm max-w-xs">
        <span className="block text-xs text-muted-ink mb-1">Applies to</span>
        <select
          value={scope}
          onChange={event => setScope(event.target.value)}
          className="w-full rounded-md border border-edge bg-surface px-2 py-1.5 text-sm text-ink"
        >
          <option value="">All clients (workspace)</option>
          {clients.map(client => (
            <option key={client.id} value={client.id}>
              {client.name} only
            </option>
          ))}
        </select>
      </label>

      <div className="bg-surface border border-edge rounded-lg divide-y divide-edge">
        {data.options.map(option => (
          <div key={option.value} className="flex items-center gap-3 px-4 py-2.5">
            <div className="flex-1 min-w-0">
              <div className="text-sm text-ink">{option.label}</div>
              <div className="text-xs text-muted-ink truncate">{option.description}</div>
            </div>
            <select
              value={currentFor(option.value)}
              disabled={busy}
              onChange={event => void setMapping(option.value, event.target.value)}
              className="rounded-md border border-edge bg-surface px-2 py-1 text-sm text-ink"
            >
              <option value={NO_MAPPING}>
                {scope === '' ? 'No mapping' : 'Use the workspace mapping'}
              </option>
              {templates.map(id => (
                <option key={id} value={id}>
                  {id}
                </option>
              ))}
            </select>
          </div>
        ))}
      </div>
    </div>
  );
}
