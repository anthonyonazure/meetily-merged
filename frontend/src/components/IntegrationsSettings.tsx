'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import {
  Building2,
  CalendarClock,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  Loader2,
  MessageSquare,
  Users,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Switch } from '@/components/ui/switch';

const INTEGRATIONS_STORE = 'integrations.json';

interface M365AuthStatus {
  connected: boolean;
  account_name: string | null;
  account_email: string | null;
}

interface M365Config {
  client_id: string;
  tenant: string;
  is_default: boolean;
}

interface DeviceLoginStart {
  user_code: string;
  verification_uri: string;
  expires_in: number;
}

interface ShareTargets {
  slack: boolean;
  teams: boolean;
}

interface ConnectedAccount {
  name: string;
  email: string;
}

async function openIntegrationsStore() {
  const { Store } = await import('@tauri-apps/plugin-store');
  return Store.load(INTEGRATIONS_STORE);
}

function SettingsCard({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <div className="bg-surface rounded-lg border border-edge p-6 shadow-sm">
      <h3 className="text-lg font-semibold text-ink mb-2">{title}</h3>
      <p className="text-sm text-muted-ink mb-4">{description}</p>
      {children}
    </div>
  );
}

// Slack / Teams webhook editor. The stored URL is a secret in the OS
// keychain and is never read back into the UI — only a configured flag is.
function WebhookField({
  kind,
  label,
  placeholder,
  configured,
  onChanged,
}: {
  kind: 'slack' | 'teams';
  label: string;
  placeholder: string;
  configured: boolean;
  onChanged: () => void;
}) {
  const [value, setValue] = useState('');
  const [busy, setBusy] = useState(false);

  const save = async () => {
    if (!value.trim()) return;
    setBusy(true);
    try {
      await invoke('share_set_webhook', { kind, url: value.trim() });
      setValue('');
      toast.success(`${label} webhook saved`);
      onChanged();
    } catch (error) {
      toast.error(`Could not save ${label} webhook`, { description: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      await invoke('share_set_webhook', { kind, url: '' });
      toast.success(`${label} webhook removed`);
      onChanged();
    } catch (error) {
      toast.error(`Could not remove ${label} webhook`, { description: String(error) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 text-sm text-muted-ink">
        {configured ? (
          <span className="inline-flex items-center gap-1 text-ink">
            <Check className="w-4 h-4" /> Webhook configured (stored in your OS keychain)
          </span>
        ) : (
          <span>No webhook configured</span>
        )}
      </div>
      <div className="flex gap-2">
        <Input
          type="password"
          value={value}
          onChange={event => setValue(event.target.value)}
          placeholder={configured ? 'Paste a new URL to replace it' : placeholder}
          className="flex-1"
          autoComplete="off"
        />
        <Button variant="outline" size="sm" disabled={busy || !value.trim()} onClick={() => void save()}>
          Save
        </Button>
        {configured && (
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void clear()}>
            Remove
          </Button>
        )}
      </div>
    </div>
  );
}

export function IntegrationsSettings() {
  // Microsoft 365 connection state
  const [m365Status, setM365Status] = useState<'loading' | 'disconnected' | 'pending' | 'connected'>('loading');
  const [account, setAccount] = useState<ConnectedAccount | null>(null);
  const [login, setLogin] = useState<DeviceLoginStart | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [config, setConfig] = useState<M365Config | null>(null);
  const [clientIdDraft, setClientIdDraft] = useState('');
  const [tenantDraft, setTenantDraft] = useState('');

  // Share targets + toggles
  const [targets, setTargets] = useState<ShareTargets>({ slack: false, teams: false });
  const [promptToJoin, setPromptToJoin] = useState(true);
  const [googleClientId, setGoogleClientId] = useState('');

  const refreshAuthStatus = useCallback(async () => {
    try {
      const status = await invoke<M365AuthStatus>('m365_auth_status');
      if (status.connected) {
        setM365Status('connected');
        setAccount({ name: status.account_name ?? 'Microsoft 365 account', email: status.account_email ?? '' });
      } else {
        setM365Status(previous => (previous === 'pending' ? previous : 'disconnected'));
        setAccount(null);
      }
    } catch (error) {
      console.error('Failed to read M365 status:', error);
      setM365Status('disconnected');
    }
  }, []);

  const refreshTargets = useCallback(async () => {
    try {
      setTargets(await invoke<ShareTargets>('share_get_targets'));
    } catch (error) {
      console.error('Failed to read share targets:', error);
    }
  }, []);

  useEffect(() => {
    void refreshAuthStatus();
    void refreshTargets();
    void (async () => {
      try {
        const loaded = await invoke<M365Config>('m365_get_config');
        setConfig(loaded);
        setClientIdDraft(loaded.is_default ? '' : loaded.client_id);
        setTenantDraft(loaded.is_default ? '' : loaded.tenant);
      } catch (error) {
        console.error('Failed to read M365 config:', error);
      }
      try {
        const store = await openIntegrationsStore();
        setPromptToJoin((await store.get<boolean>('autojoin_prompt')) ?? true);
        setGoogleClientId((await store.get<string>('google_client_id')) ?? '');
      } catch (error) {
        console.error('Failed to read integrations store:', error);
      }
    })();
  }, [refreshAuthStatus, refreshTargets]);

  // Device-login completion arrives via events from the Rust poll task.
  useEffect(() => {
    let unlistenConnected: UnlistenFn | undefined;
    let unlistenFailed: UnlistenFn | undefined;
    void (async () => {
      unlistenConnected = await listen<ConnectedAccount>('m365-connected', event => {
        setLogin(null);
        setM365Status('connected');
        setAccount(event.payload);
        toast.success('Microsoft 365 connected', { description: event.payload.email });
      });
      unlistenFailed = await listen<string>('m365-auth-failed', event => {
        setLogin(null);
        setM365Status('disconnected');
        toast.error('Microsoft 365 sign-in failed', { description: event.payload });
      });
    })();
    return () => {
      unlistenConnected?.();
      unlistenFailed?.();
    };
  }, []);

  const handleConnect = async () => {
    try {
      const start = await invoke<DeviceLoginStart>('m365_begin_device_login');
      setLogin(start);
      setM365Status('pending');
    } catch (error) {
      toast.error('Could not start Microsoft sign-in', { description: String(error) });
    }
  };

  const handleCancelLogin = async () => {
    try {
      await invoke('m365_cancel_device_login');
    } catch (error) {
      console.error('Failed to cancel login:', error);
    }
    setLogin(null);
    setM365Status('disconnected');
  };

  const handleDisconnect = async () => {
    try {
      await invoke('m365_disconnect');
      setAccount(null);
      setM365Status('disconnected');
      toast.success('Microsoft 365 disconnected');
    } catch (error) {
      toast.error('Could not disconnect', { description: String(error) });
    }
  };

  const handleCopyCode = async () => {
    if (!login) return;
    await navigator.clipboard.writeText(login.user_code);
    toast.success('Code copied');
  };

  const handleOpenVerification = async () => {
    if (!login) return;
    try {
      await invoke('open_external_url', { url: login.verification_uri });
    } catch (error) {
      toast.error('Could not open the sign-in page', { description: String(error) });
    }
  };

  const handleSaveConfig = async () => {
    try {
      const saved = await invoke<M365Config>('m365_set_config', {
        clientId: clientIdDraft.trim() || null,
        tenant: tenantDraft.trim() || null,
      });
      setConfig(saved);
      setAccount(null);
      setM365Status('disconnected');
      toast.success('Microsoft 365 app settings saved', {
        description: 'Any previous connection was signed out. Connect again to use the new registration.',
      });
    } catch (error) {
      toast.error('Could not save app settings', { description: String(error) });
    }
  };

  const handlePromptToggle = async (enabled: boolean) => {
    const previous = promptToJoin;
    setPromptToJoin(enabled);
    try {
      const store = await openIntegrationsStore();
      await store.set('autojoin_prompt', enabled);
      await store.save();
    } catch (error) {
      setPromptToJoin(previous);
      toast.error('Failed to save preference', { description: String(error) });
    }
  };

  const handleSaveGoogleClientId = async () => {
    try {
      const store = await openIntegrationsStore();
      await store.set('google_client_id', googleClientId.trim());
      await store.save();
      toast.success('Google client ID saved');
    } catch (error) {
      toast.error('Failed to save Google client ID', { description: String(error) });
    }
  };

  return (
    <div className="space-y-6 mt-6">
      {/* Microsoft 365 */}
      <SettingsCard
        title="Microsoft 365"
        description="Connect your work or school account to see your Outlook calendar in the sidebar and email meeting summaries as Outlook drafts. Meetily only reads your calendar and creates drafts — it never sends mail and never posts anything on its own."
      >
        <div className="space-y-4">
          {m365Status === 'loading' && (
            <div className="flex items-center gap-2 text-sm text-muted-ink">
              <Loader2 className="w-4 h-4 animate-spin" /> Checking connection…
            </div>
          )}

          {m365Status === 'disconnected' && (
            <Button variant="outline" size="sm" onClick={() => void handleConnect()}>
              <Building2 className="w-4 h-4" />
              Connect Microsoft 365
            </Button>
          )}

          {m365Status === 'pending' && login && (
            <div className="rounded-md border border-edge bg-app p-4 space-y-3">
              <p className="text-sm text-ink">
                Enter this code on the Microsoft sign-in page, then approve the request:
              </p>
              <div className="flex items-center gap-3">
                <span className="font-mono text-xl tracking-widest text-ink-bright select-all">
                  {login.user_code}
                </span>
                <Button variant="outline" size="sm" onClick={() => void handleCopyCode()} title="Copy code">
                  <Copy className="w-4 h-4" />
                </Button>
              </div>
              <div className="flex items-center gap-2">
                <Button variant="outline" size="sm" onClick={() => void handleOpenVerification()}>
                  <ExternalLink className="w-4 h-4" />
                  Open sign-in page
                </Button>
                <Button variant="ghost" size="sm" onClick={() => void handleCancelLogin()}>
                  Cancel
                </Button>
              </div>
              <div className="flex items-center gap-2 text-xs text-faint">
                <Loader2 className="w-3 h-3 animate-spin" />
                Waiting for you to finish signing in…
              </div>
            </div>
          )}

          {m365Status === 'connected' && account && (
            <div className="flex items-center justify-between">
              <div>
                <div className="text-sm font-medium text-ink">{account.name}</div>
                {account.email && <div className="text-xs text-muted-ink">{account.email}</div>}
              </div>
              <Button variant="outline" size="sm" onClick={() => void handleDisconnect()}>
                Disconnect
              </Button>
            </div>
          )}

          {/* Advanced: use a different Entra app registration */}
          <button
            onClick={() => setShowAdvanced(previous => !previous)}
            className="flex items-center gap-1 text-xs text-muted-ink hover:text-ink"
          >
            {showAdvanced ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
            Advanced: use your own app registration
          </button>
          {showAdvanced && (
            <div className="space-y-2 pl-4 border-l border-edge">
              <p className="text-xs text-muted-ink">
                Defaults to the built-in Meetily registration{config && !config.is_default ? ' (currently overridden)' : ''}.
                Leave a field empty to use the default. Saving signs out any current connection.
              </p>
              <Input
                value={clientIdDraft}
                onChange={event => setClientIdDraft(event.target.value)}
                placeholder={`Client ID (default: ${config?.is_default ? config.client_id : 'built-in'})`}
                autoComplete="off"
              />
              <Input
                value={tenantDraft}
                onChange={event => setTenantDraft(event.target.value)}
                placeholder="Tenant ID or domain (default: built-in)"
                autoComplete="off"
              />
              <Button variant="outline" size="sm" onClick={() => void handleSaveConfig()}>
                Save app settings
              </Button>
            </div>
          )}
        </div>
      </SettingsCard>

      {/* Calendar join prompt */}
      <SettingsCard
        title="Meeting join prompts"
        description="When a calendar event with a meeting link is about to start, show a notification and an in-app banner with a Join button. Nothing opens until you click Join."
      >
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-sm text-ink">
            <CalendarClock className="w-4 h-4 text-muted-ink" />
            Prompt to join from calendar
          </div>
          <Switch checked={promptToJoin} onCheckedChange={value => void handlePromptToggle(value)} />
        </div>
      </SettingsCard>

      {/* Slack */}
      <SettingsCard
        title="Slack"
        description="Add a Slack incoming webhook URL to enable the per-meeting “Send summary to Slack” action. Summaries are only posted when you press that button."
      >
        <WebhookField
          kind="slack"
          label="Slack"
          placeholder="https://hooks.slack.com/services/…"
          configured={targets.slack}
          onChanged={() => void refreshTargets()}
        />
      </SettingsCard>

      {/* Teams */}
      <SettingsCard
        title="Microsoft Teams"
        description="Add a Teams incoming webhook URL (channel connector or workflow) to enable the per-meeting “Send summary to Teams” action. Summaries are only posted when you press that button."
      >
        <WebhookField
          kind="teams"
          label="Teams"
          placeholder="https://…webhook.office.com/… or Power Automate URL"
          configured={targets.teams}
          onChanged={() => void refreshTargets()}
        />
      </SettingsCard>

      {/* Google Workspace (stub) */}
      <SettingsCard
        title="Google Workspace"
        description="Coming soon: Google Calendar in the sidebar and Gmail draft sharing. Google requires each app to bring its own OAuth client ID, so this stays inactive until one is configured. You can save a client ID now — when the integration ships, it will pick it up with no other setup."
      >
        <div className="flex gap-2 items-center">
          <Users className="w-4 h-4 text-faint flex-shrink-0" />
          <Input
            value={googleClientId}
            onChange={event => setGoogleClientId(event.target.value)}
            placeholder="Google OAuth client ID (optional, for later)"
            autoComplete="off"
            className="flex-1"
          />
          <Button variant="outline" size="sm" onClick={() => void handleSaveGoogleClientId()}>
            Save
          </Button>
        </div>
        <p className="text-xs text-faint mt-2 flex items-center gap-1">
          <MessageSquare className="w-3 h-3" />
          No Google connection is made until the integration ships.
        </p>
      </SettingsCard>
    </div>
  );
}
