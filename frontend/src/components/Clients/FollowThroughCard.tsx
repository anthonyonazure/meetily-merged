'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Loader2, Mail, Send } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { MarkdownLite } from '@/components/shared/MarkdownLite';
import { useConfig } from '@/contexts/ConfigContext';
import { ChaseSuggestion, FollowThroughResult } from '@/types/clients';

interface FollowThroughCardProps {
  clientId: string;
  clientName: string;
  openCommitments: number;
}

/**
 * Follow-through on the Clients page: runs the agent over the client's stale
 * open commitments and renders the nudge list. When Microsoft 365 is
 * connected, each chase gets a "Draft chase email" button — draft only, the
 * user reviews and sends in Outlook.
 */
export function FollowThroughCard({ clientId, clientName, openCommitments }: FollowThroughCardProps) {
  const { modelConfig } = useConfig();
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<FollowThroughResult | null>(null);
  const [m365Connected, setM365Connected] = useState(false);
  const [draftingFactId, setDraftingFactId] = useState<string | null>(null);

  // Reset when switching clients.
  useEffect(() => {
    setResult(null);
    setRunning(false);
  }, [clientId]);

  useEffect(() => {
    void (async () => {
      try {
        const status = await invoke<{ connected: boolean }>('m365_auth_status');
        setM365Connected(status.connected);
      } catch {
        setM365Connected(false);
      }
    })();
  }, [clientId]);

  const handleRun = useCallback(async () => {
    if (!modelConfig.provider || !modelConfig.model) {
      toast.error('Configure a summary model first', {
        description: 'Follow-through uses the same AI provider as summaries.',
      });
      return;
    }
    setRunning(true);
    try {
      const outcome = await invoke<FollowThroughResult>('client_follow_through', {
        clientId,
        modelProvider: modelConfig.provider,
        modelName: modelConfig.model,
      });
      setResult(outcome);
    } catch (error) {
      console.error('Follow-through failed:', error);
      toast.error('Follow-through failed', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setRunning(false);
    }
  }, [clientId, modelConfig.provider, modelConfig.model]);

  const handleDraft = useCallback(
    async (chase: ChaseSuggestion) => {
      setDraftingFactId(chase.fact_id);
      try {
        const draftUrl = await invoke<string>('m365_create_summary_draft', {
          subject: chase.chase_subject,
          markdown: chase.chase_message,
          recipients: null,
        });
        await invoke('open_external_url', { url: draftUrl });
        toast.success('Outlook draft created', {
          description: 'Review the draft and press Send in Outlook.',
        });
      } catch (error) {
        console.error('Failed to create chase draft:', error);
        toast.error('Could not create the Outlook draft', {
          description: error instanceof Error ? error.message : String(error),
        });
      } finally {
        setDraftingFactId(null);
      }
    },
    [],
  );

  return (
    <div className="mb-8 bg-surface border border-edge rounded-lg">
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-edge">
        <Send className="w-4 h-4 text-muted-ink" />
        <span className="text-sm font-medium text-ink">Follow-through</span>
        <span className="text-xs text-faint">
          {openCommitments > 0
            ? `${openCommitments} open commitment${openCommitments === 1 ? '' : 's'}`
            : 'no open commitments'}
        </span>
        <div className="ml-auto">
          <Button variant="outline" size="sm" disabled={running} onClick={() => void handleRun()}>
            {running ? (
              <>
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                <span>Reviewing…</span>
              </>
            ) : (
              <span>Run follow-through</span>
            )}
          </Button>
        </div>
      </div>

      {result && (
        <div className="px-4 py-3 space-y-3">
          {result.chases.length > 0 ? (
            result.chases.map(chase => (
              <div key={chase.fact_id} className="border border-edge rounded-md px-3 py-2">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-ink flex-1 min-w-0 truncate">
                    {chase.subject}
                  </span>
                  <span className="text-xs text-faint whitespace-nowrap">
                    open {chase.age_days} day{chase.age_days === 1 ? '' : 's'}
                  </span>
                  {m365Connected && (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={draftingFactId !== null}
                      onClick={() => void handleDraft(chase)}
                      title="Create an Outlook draft with this chase message"
                    >
                      {draftingFactId === chase.fact_id ? (
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      ) : (
                        <Mail className="w-3.5 h-3.5" />
                      )}
                      <span>Draft chase email</span>
                    </Button>
                  )}
                </div>
                {chase.nudge && <div className="text-sm text-muted-ink mt-0.5">{chase.nudge}</div>}
                <div className="mt-1.5 text-sm text-ink bg-wash border-l-2 border-edge rounded px-2.5 py-1.5 whitespace-pre-wrap">
                  {chase.chase_message}
                </div>
              </div>
            ))
          ) : (
            <MarkdownLite markdown={result.markdown} />
          )}
          {!m365Connected && result.chases.length > 0 && (
            <p className="text-[11px] text-faint">
              Connect Microsoft 365 in Settings → Integrations to turn a chase into an Outlook
              draft with one click. Nothing is ever sent automatically.
            </p>
          )}
        </div>
      )}
      {!result && !running && (
        <div className="px-4 py-3 text-xs text-faint">
          Reviews {clientName}&apos;s open commitments that have gone quiet (older than 3 days, or
          overdue) and suggests a chase message for each.
        </div>
      )}
    </div>
  );
}
