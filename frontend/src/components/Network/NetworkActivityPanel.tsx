'use client';

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import {
  AlertTriangle,
  Download,
  Laptop,
  Loader2,
  RefreshCw,
  ShieldCheck,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  PURPOSE_LABEL,
  exportNetworkLog,
  formatBytes,
  formatTimestamp,
  getExpectedHosts,
  getNetworkActivity,
} from '@/lib/network';
import { ExpectedHostsReport, NetworkActivity, NetworkEvent } from '@/types/network';

function EventTable({ events }: { events: NetworkEvent[] }) {
  if (events.length === 0) {
    return (
      <p className="rounded border border-edge bg-surface p-4 text-sm text-muted-ink">
        No requests recorded.
      </p>
    );
  }
  return (
    <div className="overflow-x-auto rounded border border-edge">
      <table className="w-full min-w-[52rem] text-sm">
        <thead className="bg-wash text-left text-xs uppercase tracking-wide text-muted-ink">
          <tr>
            <th className="px-3 py-2 font-medium">When</th>
            <th className="px-3 py-2 font-medium">Where</th>
            <th className="px-3 py-2 font-medium">Why</th>
            <th className="px-3 py-2 font-medium">Carried</th>
            <th className="px-3 py-2 font-medium">Sent</th>
            <th className="px-3 py-2 font-medium">Received</th>
            <th className="px-3 py-2 font-medium">Profile</th>
            <th className="px-3 py-2 font-medium">Result</th>
          </tr>
        </thead>
        <tbody className="bg-surface">
          {events.map(event => (
            <tr key={event.id} className="border-t border-edge align-top">
              <td className="px-3 py-2 whitespace-nowrap text-muted-ink">
                {formatTimestamp(event.created_at)}
              </td>
              <td className="px-3 py-2">
                <div className="text-ink">{event.host}</div>
                <div className="text-xs text-faint break-all">{event.url}</div>
              </td>
              <td className="px-3 py-2 whitespace-nowrap">
                {PURPOSE_LABEL[event.purpose] ?? event.purpose}
              </td>
              <td className="px-3 py-2 whitespace-nowrap">
                {event.carried_audio && <span className="status-chip mr-1">Audio</span>}
                {event.carried_transcript && <span className="status-chip">Transcript</span>}
                {!event.carried_audio && !event.carried_transcript && (
                  <span className="text-muted-ink">Nothing from a meeting</span>
                )}
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-muted-ink">
                {formatBytes(event.bytes_out)}
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-muted-ink">
                {formatBytes(event.bytes_in)}
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-muted-ink">
                {event.profile_name ?? 'None'}
              </td>
              <td className="px-3 py-2 whitespace-nowrap">
                {event.outcome === 'ok' ? (
                  <span className="text-muted-ink">OK</span>
                ) : (
                  <span className="text-rec" title={event.detail}>
                    Failed
                  </span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function NetworkActivityPanel() {
  const [activity, setActivity] = useState<NetworkActivity | null>(null);
  const [expected, setExpected] = useState<ExpectedHostsReport | null>(null);
  const [view, setView] = useState<'session' | 'history'>('session');
  const [exporting, setExporting] = useState(false);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [next, hosts] = await Promise.all([getNetworkActivity(), getExpectedHosts()]);
      setActivity(next);
      setExpected(hosts);
    } catch (error) {
      console.error('Failed to read network activity:', error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const runExport = useCallback(async () => {
    if (exporting) return;
    setExporting(true);
    try {
      const result = await exportNetworkLog();
      toast.success(`Exported ${result.events} request(s)`, { description: result.folder });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message !== 'cancelled') {
        toast.error('Could not export the network log', { description: message });
      }
    } finally {
      setExporting(false);
    }
  }, [exporting]);

  if (!activity || !expected) {
    return (
      <div className="flex items-center gap-2 py-8 text-muted-ink">
        <Loader2 className="w-4 h-4 animate-spin" />
        Reading the network log…
      </div>
    );
  }

  const nothingWentOut = activity.session_request_count === 0;
  const events = view === 'session' ? activity.session_events : activity.historical_events;
  const tallies = view === 'session' ? activity.session_tallies : activity.all_time_tallies;

  return (
    <div className="space-y-8 py-6">
      <section>
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h2 className="section-header text-xl mb-4">Network activity</h2>
          </div>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" disabled={loading} onClick={() => void refresh()}>
              {loading ? (
                <Loader2 className="w-4 h-4 mr-2 animate-spin" />
              ) : (
                <RefreshCw className="w-4 h-4 mr-2" />
              )}
              Refresh
            </Button>
            <Button variant="outline" size="sm" disabled={exporting} onClick={() => void runExport()}>
              <Download className="w-4 h-4 mr-2" />
              Export CSV
            </Button>
          </div>
        </div>

        <div
          className={`flex items-start gap-3 rounded border p-4 ${
            nothingWentOut ? 'border-edge bg-surface' : 'border-edge bg-wash'
          }`}
        >
          {nothingWentOut ? (
            <ShieldCheck className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
          ) : (
            <Laptop className="w-4 h-4 mt-0.5 text-muted-ink shrink-0" />
          )}
          <div>
            <p className="text-ink">{activity.headline}</p>
            <p className="mt-1 text-sm text-muted-ink">
              {activity.total_request_count} request(s) recorded in total, across every session
              since this was installed.
            </p>
          </div>
        </div>

        {activity.unexpected_hosts.length > 0 && (
          <div className="mt-4 flex items-start gap-3 rounded border border-rec bg-wash p-4">
            <AlertTriangle className="w-4 h-4 mt-0.5 text-rec shrink-0" />
            <div>
              <p className="text-ink">
                This app reached {activity.unexpected_hosts.length} host(s) that are not in its own
                expected list.
              </p>
              <p className="mt-1 text-sm text-muted-ink">
                {activity.unexpected_hosts.join(', ')}. If you configured a custom endpoint or a
                webhook, that is why. If not, it is worth asking about.
              </p>
            </div>
          </div>
        )}

        <p className="mt-4 text-xs text-muted-ink max-w-3xl">{activity.caveat}</p>
      </section>

      <section>
        <div className="mb-4 flex items-center gap-2">
          <Button
            variant={view === 'session' ? 'default' : 'outline'}
            size="sm"
            onClick={() => setView('session')}
          >
            This session ({activity.session_request_count})
          </Button>
          <Button
            variant={view === 'history' ? 'default' : 'outline'}
            size="sm"
            onClick={() => setView('history')}
          >
            History ({activity.total_request_count})
          </Button>
        </div>

        {tallies.length > 0 && (
          <div className="mb-4 flex flex-wrap gap-2">
            {tallies.map(tally => (
              <span
                key={tally.host}
                className={`rounded border px-2 py-1 text-xs ${
                  tally.expected ? 'border-edge text-muted-ink' : 'border-rec text-rec'
                }`}
                title={
                  tally.on_device
                    ? 'On this machine only — this traffic never reaches a network.'
                    : undefined
                }
              >
                {tally.host}
                {tally.on_device ? ' (on this machine)' : ''} · {tally.requests} ·{' '}
                {formatBytes(tally.bytes_out + tally.bytes_in)}
              </span>
            ))}
          </div>
        )}

        <EventTable events={events} />
      </section>

      <section>
        <h3 className="section-header text-lg mb-4">Hosts this app can ever contact</h3>
        <p className="text-sm text-muted-ink max-w-3xl">{expected.note}</p>
        <div className="mt-4 space-y-3">
          {expected.hosts.map(host => (
            <div key={host.host} className="rounded border border-edge bg-surface p-4">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium text-ink">{host.host}</span>
                <span className="status-chip">{PURPOSE_LABEL[host.purpose] ?? host.purpose}</span>
                {host.on_device && (
                  <span className="rounded border border-edge px-2 py-0.5 text-xs text-muted-ink">
                    Never leaves this machine
                  </span>
                )}
                {host.only_when_configured && (
                  <span className="rounded border border-edge px-2 py-0.5 text-xs text-muted-ink">
                    Only if you turn it on
                  </span>
                )}
              </div>
              <p className="mt-2 text-sm text-muted-ink">{host.what_for}</p>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
