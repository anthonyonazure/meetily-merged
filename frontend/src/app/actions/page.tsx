'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ExternalLink, ListChecks, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ActionItemWithMeeting } from '@/types/agents';

interface MeetingGroup {
  meetingId: string;
  meetingTitle: string;
  items: ActionItemWithMeeting[];
}

export default function ActionsPage() {
  const router = useRouter();
  const [items, setItems] = useState<ActionItemWithMeeting[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [showDone, setShowDone] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const all = await invoke<ActionItemWithMeeting[]>('actions_list');
      setItems(all);
    } catch (error) {
      console.error('Failed to load action items:', error);
      toast.error('Failed to load action items');
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const groups = useMemo<MeetingGroup[]>(() => {
    const visible = showDone ? items : items.filter(item => item.status === 'open');
    const byMeeting = new Map<string, MeetingGroup>();
    for (const item of visible) {
      let group = byMeeting.get(item.meeting_id);
      if (!group) {
        group = { meetingId: item.meeting_id, meetingTitle: item.meeting_title, items: [] };
        byMeeting.set(item.meeting_id, group);
      }
      group.items.push(item);
    }
    return [...byMeeting.values()];
  }, [items, showDone]);

  const openCount = items.filter(item => item.status === 'open').length;

  const handleToggle = useCallback(async (item: ActionItemWithMeeting) => {
    const nextStatus = item.status === 'done' ? 'open' : 'done';
    setItems(previous =>
      previous.map(a => (a.id === item.id ? { ...a, status: nextStatus } : a)));
    try {
      await invoke('action_set_status', { actionId: item.id, status: nextStatus });
    } catch (error) {
      console.error('Failed to update action item:', error);
      setItems(previous =>
        previous.map(a => (a.id === item.id ? { ...a, status: item.status } : a)));
      toast.error('Failed to update action item');
    }
  }, []);

  const handleDelete = useCallback(async (item: ActionItemWithMeeting) => {
    const previousItems = items;
    setItems(previous => previous.filter(a => a.id !== item.id));
    try {
      await invoke('action_delete', { actionId: item.id });
    } catch (error) {
      console.error('Failed to delete action item:', error);
      setItems(previousItems);
      toast.error('Failed to delete action item');
    }
  }, [items]);

  return (
    <div className="h-screen overflow-y-auto bg-gray-50">
      <div className="max-w-3xl mx-auto p-8">
        <div className="flex items-center gap-3 mb-1">
          <ListChecks className="w-6 h-6 text-gray-600" />
          <h1 className="text-2xl font-semibold text-gray-900">Actions</h1>
        </div>
        <p className="text-sm text-gray-500 mb-6">
          Action items extracted from your meetings by the Action Tracker agent.
          {openCount > 0 ? ` ${openCount} open.` : ''}
        </p>

        <label className="flex items-center gap-2 text-sm text-gray-600 mb-4 cursor-pointer">
          <input
            type="checkbox"
            checked={showDone}
            onChange={event => setShowDone(event.target.checked)}
          />
          Show completed items
        </label>

        {isLoading ? (
          <div className="text-sm text-gray-400 py-8 text-center">Loading action items…</div>
        ) : groups.length === 0 ? (
          <div className="text-sm text-gray-400 py-8 text-center border border-dashed border-gray-200 rounded-lg bg-white">
            {showDone
              ? 'No action items yet. Run the Action Tracker agent on a meeting, or let it run automatically after a summary.'
              : 'No open action items. Nice work.'}
          </div>
        ) : (
          <div className="space-y-6">
            {groups.map(group => (
              <div key={group.meetingId} className="bg-white border border-gray-200 rounded-lg">
                <div className="flex items-center justify-between px-4 py-2.5 border-b border-gray-100">
                  <span className="text-sm font-medium text-gray-800 truncate">
                    {group.meetingTitle}
                  </span>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => router.push(`/meeting-details?id=${group.meetingId}`)}
                  >
                    <ExternalLink className="w-3.5 h-3.5" />
                    <span>Open meeting</span>
                  </Button>
                </div>
                <div className="px-4 py-2 space-y-1.5">
                  {group.items.map(item => (
                    <div key={item.id} className="flex items-start gap-2 group">
                      <input
                        type="checkbox"
                        checked={item.status === 'done'}
                        onChange={() => void handleToggle(item)}
                        className="mt-1 cursor-pointer"
                      />
                      <span
                        className={`flex-1 text-sm ${item.status === 'done' ? 'line-through text-gray-400' : 'text-gray-700'}`}
                      >
                        {item.description}
                        {item.owner ? <span className="text-gray-400"> · {item.owner}</span> : null}
                        {item.due_hint ? <span className="text-gray-400"> · {item.due_hint}</span> : null}
                      </span>
                      <button
                        onClick={() => void handleDelete(item)}
                        className="opacity-0 group-hover:opacity-100 transition-opacity text-gray-400 hover:text-red-500 mt-0.5"
                        title="Delete action item"
                        aria-label="Delete action item"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
