'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import {
  Briefcase,
  Check,
  CircleDollarSign,
  ExternalLink,
  Gavel,
  ListTodo,
  RotateCcw,
  StickyNote,
  Trash2,
  X,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  ClientTimeline,
  ClientWithCounts,
  MemoryFact,
  MemoryFactWithMeeting,
} from '@/types/clients';
import { FollowThroughCard } from '@/components/Clients/FollowThroughCard';
import { ChatPanel } from '@/components/MeetingDetails/ChatPanel';
import { ProfilePicker } from '@/components/Privacy/ProfilePicker';
const KIND_META: Record<string, { label: string; icon: typeof ListTodo }> = {
  commitment: { label: 'Commitment', icon: ListTodo },
  decision: { label: 'Decision', icon: Gavel },
  figure: { label: 'Figure', icon: CircleDollarSign },
  note: { label: 'Note', icon: StickyNote },
};

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

interface TimelineGroup {
  meetingId: string;
  title: string;
  createdAt: string;
  facts: MemoryFactWithMeeting[];
}

function FactRow({
  fact,
  onSetStatus,
  onDelete,
}: {
  fact: MemoryFact;
  onSetStatus: (fact: MemoryFact, status: string) => void;
  onDelete: (fact: MemoryFact) => void;
}) {
  const meta = KIND_META[fact.kind] ?? KIND_META.note;
  const Icon = meta.icon;
  const isCommitment = fact.kind === 'commitment';
  const isOpen = isCommitment && fact.status === 'open';
  const isResolved = fact.status === 'done' || fact.status === 'dismissed';

  return (
    <div
      className={`group flex items-start gap-2.5 rounded-md border px-3 py-2 ${
        isOpen
          ? 'border-l-2 border-edge border-l-rec bg-wash'
          : 'border-edge bg-surface'
      } ${isResolved ? 'opacity-60' : ''}`}
    >
      <Icon className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-ink" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 flex-wrap">
          <span
            className={`text-sm font-medium ${
              fact.status === 'done' ? 'line-through text-faint' : 'text-ink'
            }`}
          >
            {fact.subject}
          </span>
          <span className="text-[10px] uppercase tracking-wide text-faint">{meta.label}</span>
          {fact.status === 'dismissed' && (
            <span className="text-[10px] uppercase tracking-wide text-faint">dismissed</span>
          )}
        </div>
        <div className="text-sm text-muted-ink">{fact.detail}</div>
        <div className="text-xs text-faint">
          {fact.owner ? `${fact.owner}` : null}
          {fact.owner && (fact.due_hint || fact.amount) ? ' · ' : null}
          {fact.due_hint ? `due ${fact.due_hint}` : null}
          {fact.due_hint && fact.amount ? ' · ' : null}
          {fact.amount ?? null}
        </div>
      </div>
      <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
        {isCommitment && fact.status !== 'done' && (
          <button
            onClick={() => onSetStatus(fact, 'done')}
            className="p-1 rounded text-muted-ink hover:text-green-700 hover:bg-active"
            title="Mark done"
            aria-label="Mark commitment done"
          >
            <Check className="w-3.5 h-3.5" />
          </button>
        )}
        {isCommitment && fact.status === 'open' && (
          <button
            onClick={() => onSetStatus(fact, 'dismissed')}
            className="p-1 rounded text-muted-ink hover:text-ink hover:bg-active"
            title="Dismiss"
            aria-label="Dismiss commitment"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
        {isCommitment && isResolved && (
          <button
            onClick={() => onSetStatus(fact, 'open')}
            className="p-1 rounded text-muted-ink hover:text-ink hover:bg-active"
            title="Reopen"
            aria-label="Reopen commitment"
          >
            <RotateCcw className="w-3.5 h-3.5" />
          </button>
        )}
        <button
          onClick={() => onDelete(fact)}
          className="p-1 rounded text-faint hover:text-red-600 hover:bg-active"
          title="Delete fact"
          aria-label="Delete fact"
        >
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  );
}

export default function ClientsPage() {
  const router = useRouter();
  const [clients, setClients] = useState<ClientWithCounts[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<ClientTimeline | null>(null);
  const [timelineLoading, setTimelineLoading] = useState(false);

  const refreshClients = useCallback(async () => {
    try {
      const list = await invoke<ClientWithCounts[]>('clients_list');
      setClients(list);
      return list;
    } catch (error) {
      console.error('Failed to load clients:', error);
      toast.error('Failed to load clients');
      return [];
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshClients();
  }, [refreshClients]);

  const loadTimeline = useCallback(async (clientId: string) => {
    setTimelineLoading(true);
    try {
      const data = await invoke<ClientTimeline>('client_timeline', { clientId });
      setTimeline(data);
    } catch (error) {
      console.error('Failed to load client timeline:', error);
      toast.error('Failed to load client timeline');
      setTimeline(null);
    } finally {
      setTimelineLoading(false);
    }
  }, []);

  useEffect(() => {
    if (selectedId) void loadTimeline(selectedId);
    else setTimeline(null);
  }, [selectedId, loadTimeline]);

  const groups = useMemo<TimelineGroup[]>(() => {
    if (!timeline) return [];
    const byMeeting = new Map<string, TimelineGroup>();
    for (const meeting of timeline.meetings) {
      byMeeting.set(meeting.id, {
        meetingId: meeting.id,
        title: meeting.title,
        createdAt: meeting.created_at,
        facts: [],
      });
    }
    for (const fact of timeline.facts) {
      let group = byMeeting.get(fact.meeting_id);
      if (!group) {
        // Fact from a meeting that was later untagged: still part of memory.
        group = {
          meetingId: fact.meeting_id,
          title: fact.meeting_title,
          createdAt: fact.meeting_created_at,
          facts: [],
        };
        byMeeting.set(fact.meeting_id, group);
      }
      group.facts.push(fact);
    }
    return [...byMeeting.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  }, [timeline]);

  const handleSetStatus = useCallback(
    async (fact: MemoryFact, status: string) => {
      try {
        await invoke('memory_fact_set_status', { factId: fact.id, status });
        if (selectedId) await loadTimeline(selectedId);
        void refreshClients();
      } catch (error) {
        console.error('Failed to update memory fact:', error);
        toast.error('Failed to update commitment');
      }
    },
    [selectedId, loadTimeline, refreshClients],
  );

  const handleDeleteFact = useCallback(
    async (fact: MemoryFact) => {
      try {
        await invoke('memory_fact_delete', { factId: fact.id });
        if (selectedId) await loadTimeline(selectedId);
        void refreshClients();
      } catch (error) {
        console.error('Failed to delete memory fact:', error);
        toast.error('Failed to delete fact');
      }
    },
    [selectedId, loadTimeline, refreshClients],
  );

  const selectedClient = clients.find(client => client.id === selectedId) ?? null;

  return (
    <div className="h-screen overflow-hidden bg-app flex">
      {/* Client list */}
      <div className="w-72 flex-shrink-0 border-r border-edge bg-surface flex flex-col">
        <div className="p-4 border-b border-edge">
          <div className="flex items-center gap-2">
            <Briefcase className="w-5 h-5 text-muted-ink" />
            <h1 className="text-xl font-display font-semibold text-ink">Clients</h1>
          </div>
          <p className="text-xs text-muted-ink mt-1">
            Every tagged meeting feeds a client&apos;s running memory.
          </p>
        </div>
        <div className="flex-1 overflow-y-auto custom-scrollbar p-2 space-y-1">
          {isLoading ? (
            <div className="text-sm text-faint p-3">Loading…</div>
          ) : clients.length === 0 ? (
            <div className="text-sm text-faint p-3">
              No clients yet. Add them in Settings → Clients, or tag a meeting from its header.
            </div>
          ) : (
            clients.map(client => (
              <button
                key={client.id}
                onClick={() => setSelectedId(client.id)}
                className={`w-full text-left rounded-lg px-3 py-2.5 transition-colors ${
                  selectedId === client.id ? 'bg-active' : 'hover:bg-wash'
                }`}
              >
                <div className="text-sm font-medium text-ink truncate">{client.name}</div>
                <div className="text-xs text-muted-ink">
                  {client.meeting_count} meeting{client.meeting_count === 1 ? '' : 's'}
                  {client.open_commitments > 0 && (
                    <span className="ml-2 status-chip">{client.open_commitments} open</span>
                  )}
                </div>
              </button>
            ))
          )}
        </div>
      </div>

      {/* Timeline */}
      <div className="flex-1 overflow-y-auto custom-scrollbar">
        {!selectedClient ? (
          <div className="h-full flex items-center justify-center text-sm text-faint">
            Select a client to see their memory timeline.
          </div>
        ) : (
          <div className="max-w-3xl mx-auto p-8">
            <h2 className="text-2xl font-display font-semibold text-ink mb-0.5">
              {selectedClient.name}
            </h2>
            <p className="text-sm text-muted-ink mb-6">
              {selectedClient.domain ? `@${selectedClient.domain} · ` : ''}
              {selectedClient.meeting_count} meeting{selectedClient.meeting_count === 1 ? '' : 's'}
              {selectedClient.open_commitments > 0
                ? ` · ${selectedClient.open_commitments} open commitment${selectedClient.open_commitments === 1 ? '' : 's'}`
                : ''}
            </p>

            {/* Which privacy profile governs this client's meetings. */}
            <div className="bg-surface border border-edge rounded-lg p-4 mb-6">
              <ProfilePicker
                clientId={selectedClient.id}
                clientName={selectedClient.name}
                profileId={selectedClient.privacy_profile_id}
                onChange={() => void refreshClients()}
              />
            </div>

            <FollowThroughCard
              clientId={selectedClient.id}
              clientName={selectedClient.name}
              openCommitments={selectedClient.open_commitments}
            />

            {timelineLoading && !timeline ? (
              <div className="text-sm text-faint py-8 text-center">Loading timeline…</div>
            ) : groups.length === 0 ? (
              <div className="text-sm text-faint py-8 text-center border border-dashed border-edge rounded-lg bg-surface">
                No meetings tagged with this client yet. Tag one from the meeting header and its
                memory will collect here.
              </div>
            ) : (
              <div className="space-y-8">
                {groups.map(group => (
                  <section key={group.meetingId}>
                    <div className="flex items-baseline justify-between gap-3 mb-3">
                      <h3 className="text-lg font-display font-semibold section-header flex-1 min-w-0">
                        <span className="truncate">{group.title}</span>
                      </h3>
                      <span className="text-xs text-faint whitespace-nowrap">
                        {formatDate(group.createdAt)}
                      </span>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => router.push(`/meeting-details?id=${group.meetingId}`)}
                        title="Open meeting"
                        aria-label={`Open meeting ${group.title}`}
                      >
                        <ExternalLink className="w-3.5 h-3.5" />
                      </Button>
                    </div>
                    {group.facts.length === 0 ? (
                      <div className="text-xs text-faint px-1">
                        No memory extracted from this meeting yet.
                      </div>
                    ) : (
                      <div className="space-y-1.5">
                        {group.facts.map(fact => (
                          <FactRow
                            key={fact.id}
                            fact={fact}
                            onSetStatus={(f, status) => void handleSetStatus(f, status)}
                            onDelete={f => void handleDeleteFact(f)}
                          />
                        ))}
                      </div>
                    )}
                  </section>
                ))}
              </div>
            )}

            {/* Client-scoped chat: same thread whether opened here or from a
                tagged meeting's chat panel. */}
            <div className="mt-8 border border-edge rounded-lg overflow-hidden">
              <ChatPanel
                key={selectedClient.id}
                fixedClient={{ id: selectedClient.id, name: selectedClient.name }}
              />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
