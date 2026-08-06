'use client';

import { RefObject, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Loader2, Mail, MessageSquare, Share2, Users } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Summary } from '@/types';
import { isProfileBlocked, meetingPrivacyProfile, profileBlockedMessage } from '@/lib/privacy';
import type { MeetingProfileView } from '@/types/privacy';
import { BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';

interface ShareSummaryMenuProps {
  meetingId: string;
  meetingTitle: string;
  aiSummary: Summary | null;
  summaryRef: RefObject<BlockNoteSummaryViewRef>;
}

interface ShareTargets {
  slack: boolean;
  teams: boolean;
}

type ShareAction = 'outlook' | 'slack' | 'teams';

// Extracts the summary as markdown, preferring the live editor content.
// Mirrors the fallback chain in useCopyOperations.handleCopySummary.
async function summaryMarkdown(
  summaryRef: RefObject<BlockNoteSummaryViewRef>,
  aiSummary: Summary | null,
): Promise<string> {
  if (summaryRef.current?.getMarkdown) {
    const markdown = await summaryRef.current.getMarkdown();
    if (markdown.trim()) return markdown;
  }
  if (aiSummary && 'markdown' in aiSummary && (aiSummary as any).markdown) {
    return (aiSummary as any).markdown as string;
  }
  if (aiSummary) {
    return Object.entries(aiSummary)
      .filter(([key]) => !['markdown', 'summary_json', '_section_order', 'MeetingName'].includes(key))
      .map(([, section]) => {
        if (section && typeof section === 'object' && 'title' in section && 'blocks' in section) {
          const blocks = (section as { blocks: Array<{ content: string }> }).blocks;
          return `## ${(section as { title: string }).title}\n\n${blocks.map(b => `- ${b.content}`).join('\n')}`;
        }
        return '';
      })
      .filter(s => s.trim())
      .join('\n\n');
  }
  return '';
}

// Per-meeting share actions. All three are explicit user actions: Outlook
// creates a DRAFT the user reviews and sends themselves; Slack/Teams post
// only when the button is pressed.
export function ShareSummaryMenu({
  meetingId,
  meetingTitle,
  aiSummary,
  summaryRef,
}: ShareSummaryMenuProps) {
  const [open, setOpen] = useState(false);
  const [m365Connected, setM365Connected] = useState(false);
  const [targets, setTargets] = useState<ShareTargets>({ slack: false, teams: false });
  const [busy, setBusy] = useState<ShareAction | null>(null);
  // The meeting's privacy profile can turn all three actions off. Rust refuses
  // them regardless; this is so the menu says why before anyone clicks.
  const [profile, setProfile] = useState<MeetingProfileView | null>(null);

  // Refresh availability when the menu opens (cheap local reads).
  useEffect(() => {
    if (!open) return;
    void (async () => {
      try {
        const status = await invoke<{ connected: boolean }>('m365_auth_status');
        setM365Connected(status.connected);
      } catch {
        setM365Connected(false);
      }
      try {
        setTargets(await invoke<ShareTargets>('share_get_targets'));
      } catch {
        setTargets({ slack: false, teams: false });
      }
      try {
        setProfile(await meetingPrivacyProfile(meetingId));
      } catch {
        setProfile(null);
      }
    })();
  }, [open, meetingId]);

  const runShare = async (action: ShareAction) => {
    setBusy(action);
    try {
      const markdown = await summaryMarkdown(summaryRef, aiSummary);
      if (!markdown.trim()) {
        toast.error('No summary content to share');
        return;
      }
      const title = meetingTitle || 'Meeting summary';
      if (action === 'outlook') {
        const draftUrl = await invoke<string>('m365_create_summary_draft', {
          subject: `Meeting summary: ${title}`,
          markdown,
          recipients: null,
          meetingId,
          clientId: null,
        });
        await invoke('open_external_url', { url: draftUrl });
        toast.success('Outlook draft created', {
          description: 'Review the draft and press Send in Outlook.',
        });
      } else if (action === 'slack') {
        await invoke('share_slack', { title, markdown, meetingId });
        toast.success('Summary sent to Slack');
      } else {
        await invoke('share_teams', { title, markdown, meetingId });
        toast.success('Summary sent to Teams');
      }
      setOpen(false);
    } catch (error) {
      const labels: Record<ShareAction, string> = {
        outlook: 'Could not create the Outlook draft',
        slack: 'Could not send to Slack',
        teams: 'Could not send to Teams',
      };
      toast.error(labels[action], {
        description: isProfileBlocked(error) ? profileBlockedMessage(error) : String(error),
      });
    } finally {
      setBusy(null);
    }
  };

  const sharingAllowed = profile?.profile ? profile.profile.allow_sharing : true;
  const profileHint = profile?.profile
    ? `The "${profile.profile.name}" profile has sharing turned off for this meeting`
    : '';

  const items: Array<{
    action: ShareAction;
    label: string;
    icon: typeof Mail;
    enabled: boolean;
    hint: string;
  }> = [
    {
      action: 'outlook',
      label: 'Email via Outlook (draft)',
      icon: Mail,
      enabled: sharingAllowed && m365Connected,
      hint: sharingAllowed ? 'Connect Microsoft 365 in Settings → Integrations' : profileHint,
    },
    {
      action: 'slack',
      label: 'Send to Slack',
      icon: MessageSquare,
      enabled: sharingAllowed && targets.slack,
      hint: sharingAllowed ? 'Add a Slack webhook in Settings → Integrations' : profileHint,
    },
    {
      action: 'teams',
      label: 'Send to Teams',
      icon: Users,
      enabled: sharingAllowed && targets.teams,
      hint: sharingAllowed ? 'Add a Teams webhook in Settings → Integrations' : profileHint,
    },
  ];

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" title="Share summary" aria-label="Share summary">
          <Share2 />
          <span className="hidden lg:inline">Share</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-64 p-1">
        {items.map(({ action, label, icon: Icon, enabled, hint }) => (
          <button
            key={action}
            onClick={() => enabled && busy === null && void runShare(action)}
            disabled={!enabled || busy !== null}
            title={enabled ? label : hint}
            className={`w-full flex items-center gap-2 px-3 py-2 text-sm rounded-md text-left ${
              enabled
                ? 'text-ink hover:bg-active cursor-pointer'
                : 'text-faint cursor-not-allowed'
            }`}
          >
            {busy === action ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Icon className="w-4 h-4" />
            )}
            <span className="flex-1">{label}</span>
          </button>
        ))}
        <p className="px-3 py-1.5 text-[11px] text-faint border-t border-edge mt-1">
          {sharingAllowed
            ? 'Sharing only happens when you click an action here.'
            : profileHint + '. Copy and export still work.'}
        </p>
      </PopoverContent>
    </Popover>
  );
}
