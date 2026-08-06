'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Briefcase, Pencil, Plus, Trash2, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { Client, ClientWithCounts } from '@/types/clients';
import { ProfilePicker } from '@/components/Privacy/ProfilePicker';

interface EditorState {
  id: string | null; // null = creating
  name: string;
  domain: string;
  notes: string;
}

const EMPTY_EDITOR: EditorState = { id: null, name: '', domain: '', notes: '' };

/**
 * Client registry management (Settings → Clients): create, rename, set the
 * email domain used for attendee matching, and delete. Deleting a client
 * unlinks its meetings but never deletes them.
 */
export function ClientsSettings() {
  const [clients, setClients] = useState<ClientWithCounts[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [saving, setSaving] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ClientWithCounts | null>(null);

  const refresh = useCallback(async () => {
    try {
      setClients(await invoke<ClientWithCounts[]>('clients_list'));
    } catch (error) {
      console.error('Failed to load clients:', error);
      toast.error('Failed to load clients');
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleSave = useCallback(async () => {
    if (!editor) return;
    const name = editor.name.trim();
    if (!name) {
      toast.error('Client name cannot be empty');
      return;
    }
    setSaving(true);
    try {
      const payload = {
        name,
        domain: editor.domain.trim() || null,
        notes: editor.notes.trim() || null,
      };
      if (editor.id) {
        await invoke<Client>('client_update', { clientId: editor.id, ...payload });
      } else {
        await invoke<Client>('client_create', payload);
      }
      setEditor(null);
      await refresh();
    } catch (error) {
      console.error('Failed to save client:', error);
      toast.error('Failed to save client', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSaving(false);
    }
  }, [editor, refresh]);

  const handleDelete = useCallback(async () => {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleteTarget(null);
    try {
      await invoke('client_delete', { clientId: target.id });
      await refresh();
    } catch (error) {
      console.error('Failed to delete client:', error);
      toast.error('Failed to delete client', {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  }, [deleteTarget, refresh]);

  return (
    <div className="max-w-2xl py-6">
      <div className="flex items-center gap-3 mb-1">
        <Briefcase className="w-5 h-5 text-muted-ink" />
        <h2 className="text-xl font-display font-semibold text-ink">Clients</h2>
      </div>
      <p className="text-sm text-muted-ink mb-6">
        Meetings tagged with a client build that client&apos;s memory: commitments, decisions,
        and figures collected across meetings. The email domain (like acme.com) lets Meetily
        suggest the right client from calendar attendees. The privacy profile decides where their
        meetings may be transcribed and summarised, what happens before recording, whether summaries
        can be shared, and how long they are kept.
      </p>

      {isLoading ? (
        <div className="text-sm text-faint py-6">Loading clients…</div>
      ) : (
        <div className="space-y-2">
          {clients.map(client => (
            <div
              key={client.id}
              className="flex items-center gap-3 bg-surface border border-edge rounded-lg px-4 py-3"
            >
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium text-ink truncate">{client.name}</div>
                <div className="text-xs text-muted-ink truncate">
                  {client.domain ? `@${client.domain}` : 'No domain set'}
                  {' · '}
                  {client.meeting_count} meeting{client.meeting_count === 1 ? '' : 's'}
                </div>
              </div>
              {/* The profile that governs this client's meetings. */}
              <ProfilePicker
                clientId={client.id}
                clientName={client.name}
                profileId={client.privacy_profile_id}
                layout="inline"
                onChange={() => void refresh()}
              />
              <Button
                variant="ghost"
                size="sm"
                title="Edit client"
                aria-label={`Edit ${client.name}`}
                onClick={() =>
                  setEditor({
                    id: client.id,
                    name: client.name,
                    domain: client.domain ?? '',
                    notes: client.notes,
                  })
                }
              >
                <Pencil className="w-4 h-4 text-muted-ink" />
              </Button>
              <Button
                variant="ghost"
                size="sm"
                title="Delete client"
                aria-label={`Delete ${client.name}`}
                onClick={() => setDeleteTarget(client)}
              >
                <Trash2 className="w-4 h-4 text-muted-ink hover:text-red-600" />
              </Button>
            </div>
          ))}
          {clients.length === 0 && (
            <div className="text-sm text-faint py-6 text-center border border-dashed border-edge rounded-lg bg-surface">
              No clients yet. Add your first client to start building per-client memory.
            </div>
          )}
        </div>
      )}

      {editor ? (
        <div className="mt-4 bg-surface border border-edge rounded-lg p-4 space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-ink">
              {editor.id ? 'Edit client' : 'New client'}
            </span>
            <button
              onClick={() => setEditor(null)}
              className="text-faint hover:text-muted-ink"
              title="Close"
              aria-label="Close editor"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
          <div>
            <label className="block text-xs font-medium text-muted-ink mb-1">Name</label>
            <input
              value={editor.name}
              onChange={event => setEditor({ ...editor, name: event.target.value })}
              placeholder="Acme Corp"
              autoFocus
              className="w-full rounded-md border border-edge bg-surface px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-muted-ink mb-1">
              Email domain (optional)
            </label>
            <input
              value={editor.domain}
              onChange={event => setEditor({ ...editor, domain: event.target.value })}
              placeholder="acme.com"
              className="w-full rounded-md border border-edge bg-surface px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-muted-ink mb-1">Notes (optional)</label>
            <textarea
              value={editor.notes}
              onChange={event => setEditor({ ...editor, notes: event.target.value })}
              rows={2}
              className="w-full resize-none rounded-md border border-edge bg-surface px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-blue-300"
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={() => setEditor(null)}>
              Cancel
            </Button>
            <Button variant="outline" size="sm" disabled={saving} onClick={() => void handleSave()}>
              {editor.id ? 'Save changes' : 'Create client'}
            </Button>
          </div>
        </div>
      ) : (
        <Button variant="outline" size="sm" className="mt-4" onClick={() => setEditor(EMPTY_EDITOR)}>
          <Plus className="w-4 h-4" />
          <span>Add client</span>
        </Button>
      )}

      <ConfirmationModal
        isOpen={deleteTarget !== null}
        text={`Delete ${deleteTarget?.name ?? 'this client'}? Their meetings are kept, only the client and its links are removed.`}
        onConfirm={() => void handleDelete()}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  );
}
