'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Bot, ChevronDown, ChevronUp, Check, Copy, Loader2, Play, Settings2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover';
import { MarkdownLite } from '@/components/shared/MarkdownLite';
import { useConfig } from '@/contexts/ConfigContext';
import { ActionItem, AgentInfo, AgentRun } from '@/types/agents';

const POLL_INTERVAL_MS = 4000;

interface AgentsPanelProps {
  meetingId: string;
}

export function AgentsPanel({ meetingId }: AgentsPanelProps) {
  const { modelConfig } = useConfig();
  const [expanded, setExpanded] = useState(false);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [actionItems, setActionItems] = useState<ActionItem[]>([]);
  const [startingAgentId, setStartingAgentId] = useState<string | null>(null);
  const [copiedRunId, setCopiedRunId] = useState<string | null>(null);
  const meetingIdRef = useRef(meetingId);
  meetingIdRef.current = meetingId;

  const refresh = useCallback(async () => {
    const requestedMeetingId = meetingIdRef.current;
    try {
      const [agentList, runList, itemList] = await Promise.all([
        invoke<AgentInfo[]>('agents_list'),
        invoke<AgentRun[]>('agent_runs_for_meeting', { meetingId: requestedMeetingId }),
        invoke<ActionItem[]>('actions_for_meeting', { meetingId: requestedMeetingId }),
      ]);
      // Ignore stale responses after a meeting switch.
      if (meetingIdRef.current !== requestedMeetingId) return;
      setAgents(agentList);
      setRuns(runList);
      setActionItems(itemList);
    } catch (error) {
      console.error('Failed to load agents state:', error);
    }
  }, []);

  // Initial load and reload on meeting change.
  useEffect(() => {
    setRuns([]);
    setActionItems([]);
    void refresh();
  }, [meetingId, refresh]);

  // Poll while the panel is expanded or a run is in flight (auto-runs started
  // by the backend after summary completion surface through this same poll).
  const hasRunningRun = runs.some(run => run.status === 'running');
  useEffect(() => {
    if (!expanded && !hasRunningRun) return;
    const interval = setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [expanded, hasRunningRun, refresh]);

  const handleRun = useCallback(async (agent: AgentInfo) => {
    if (!modelConfig.provider || !modelConfig.model) {
      toast.error('Configure a summary model first', {
        description: 'Agents use the same AI provider as summaries.',
      });
      return;
    }
    setStartingAgentId(agent.id);
    try {
      await invoke('agent_run', {
        meetingId,
        agentId: agent.id,
        modelProvider: modelConfig.provider,
        modelName: modelConfig.model,
      });
      await refresh();
    } catch (error) {
      console.error(`Failed to run agent ${agent.id}:`, error);
      toast.error(`Failed to run ${agent.name}`, {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setStartingAgentId(null);
    }
  }, [meetingId, modelConfig.provider, modelConfig.model, refresh]);

  const handleToggle = useCallback(async (
    agent: AgentInfo,
    field: 'enabled' | 'auto_run',
    value: boolean,
  ) => {
    try {
      const updated = await invoke<AgentInfo>('agents_set_enabled', {
        agentId: agent.id,
        enabled: field === 'enabled' ? value : null,
        autoRun: field === 'auto_run' ? value : null,
      });
      setAgents(previous => previous.map(a => (a.id === updated.id ? updated : a)));
    } catch (error) {
      console.error('Failed to save agent setting:', error);
      toast.error('Failed to save agent setting');
    }
  }, []);

  const handleCopy = useCallback(async (run: AgentRun) => {
    if (!run.output_md) return;
    try {
      await navigator.clipboard.writeText(run.output_md);
      setCopiedRunId(run.id);
      setTimeout(() => setCopiedRunId(current => (current === run.id ? null : current)), 2000);
    } catch (error) {
      console.error('Failed to copy agent output:', error);
      toast.error('Failed to copy to clipboard');
    }
  }, []);

  const handleActionToggle = useCallback(async (item: ActionItem) => {
    const nextStatus = item.status === 'done' ? 'open' : 'done';
    // Optimistic flip; revert on failure.
    setActionItems(previous =>
      previous.map(a => (a.id === item.id ? { ...a, status: nextStatus } : a)));
    try {
      await invoke('action_set_status', { actionId: item.id, status: nextStatus });
    } catch (error) {
      console.error('Failed to update action item:', error);
      setActionItems(previous =>
        previous.map(a => (a.id === item.id ? { ...a, status: item.status } : a)));
      toast.error('Failed to update action item');
    }
  }, []);

  const latestRunFor = (agentId: string): AgentRun | undefined =>
    runs.find(run => run.agent_id === agentId); // runs are newest-first

  const runningCount = runs.filter(run => run.status === 'running').length;

  return (
    <div className="border-t border-gray-200 bg-white flex-shrink-0 flex flex-col max-h-[45%]">
      {/* Header bar */}
      <button
        onClick={() => setExpanded(previous => !previous)}
        className="flex items-center gap-2 px-4 py-2.5 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors w-full text-left"
      >
        <Bot className="w-4 h-4 text-gray-500" />
        <span>Agents</span>
        {runningCount > 0 && (
          <span className="flex items-center gap-1 text-xs text-blue-600">
            <Loader2 className="w-3 h-3 animate-spin" />
            {runningCount === 1 ? 'Running' : `${runningCount} running`}
          </span>
        )}
        {actionItems.filter(item => item.status === 'open').length > 0 && (
          <span className="text-xs text-gray-400">
            {actionItems.filter(item => item.status === 'open').length} open action item{actionItems.filter(item => item.status === 'open').length === 1 ? '' : 's'}
          </span>
        )}
        <span className="ml-auto text-gray-400">
          {expanded ? <ChevronDown className="w-4 h-4" /> : <ChevronUp className="w-4 h-4" />}
        </span>
      </button>

      {expanded && (
        <div className="overflow-y-auto px-4 pb-4 space-y-4">
          {agents.map(agent => {
            const latestRun = latestRunFor(agent.id);
            const isRunning = latestRun?.status === 'running' || startingAgentId === agent.id;
            return (
              <div key={agent.id} className="border border-gray-200 rounded-lg">
                <div className="flex items-center gap-2 px-3 py-2 border-b border-gray-100">
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium text-gray-800">{agent.name}</div>
                    <div className="text-xs text-gray-500 truncate" title={agent.description}>
                      {agent.description}
                    </div>
                  </div>
                  <Popover>
                    <PopoverTrigger asChild>
                      <Button variant="ghost" size="sm" title={`${agent.name} settings`} aria-label={`${agent.name} settings`}>
                        <Settings2 className="w-4 h-4 text-gray-500" />
                      </Button>
                    </PopoverTrigger>
                    <PopoverContent align="end" className="w-64 space-y-3">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-sm text-gray-700">Enabled</span>
                        <Switch
                          checked={agent.enabled}
                          onCheckedChange={(value: boolean) => void handleToggle(agent, 'enabled', value)}
                        />
                      </div>
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-sm text-gray-700">Auto-run after summary</span>
                        <Switch
                          checked={agent.auto_run}
                          disabled={!agent.enabled}
                          onCheckedChange={(value: boolean) => void handleToggle(agent, 'auto_run', value)}
                        />
                      </div>
                    </PopoverContent>
                  </Popover>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={!agent.enabled || isRunning}
                    onClick={() => void handleRun(agent)}
                  >
                    {isRunning ? (
                      <>
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                        <span>Running</span>
                      </>
                    ) : (
                      <>
                        <Play className="w-3.5 h-3.5" />
                        <span>Run</span>
                      </>
                    )}
                  </Button>
                </div>

                {/* Latest run output */}
                {latestRun && latestRun.status === 'error' && (
                  <div className="px-3 py-2 text-sm text-red-600">
                    {latestRun.error || 'Agent run failed'}
                  </div>
                )}
                {latestRun && latestRun.status === 'completed' && latestRun.output_md && (
                  <div className="px-3 py-2">
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-xs text-gray-400">
                        {new Date(latestRun.created_at).toLocaleString()}
                      </span>
                      <Button variant="ghost" size="sm" onClick={() => void handleCopy(latestRun)}>
                        {copiedRunId === latestRun.id ? (
                          <>
                            <Check className="w-3.5 h-3.5 text-green-600" />
                            <span>Copied</span>
                          </>
                        ) : (
                          <>
                            <Copy className="w-3.5 h-3.5" />
                            <span>Copy</span>
                          </>
                        )}
                      </Button>
                    </div>
                    <MarkdownLite markdown={latestRun.output_md} />
                  </div>
                )}

                {/* Tracked action items live under the Action Tracker card */}
                {agent.id === 'action_tracker' && actionItems.length > 0 && (
                  <div className="px-3 py-2 border-t border-gray-100 space-y-1">
                    <div className="text-xs font-medium text-gray-500 mb-1">Tracked items</div>
                    {actionItems.map(item => (
                      <label key={item.id} className="flex items-start gap-2 text-sm cursor-pointer">
                        <input
                          type="checkbox"
                          checked={item.status === 'done'}
                          onChange={() => void handleActionToggle(item)}
                          className="mt-1"
                        />
                        <span className={item.status === 'done' ? 'line-through text-gray-400' : 'text-gray-700'}>
                          {item.description}
                          {item.owner ? <span className="text-gray-400"> · {item.owner}</span> : null}
                          {item.due_hint ? <span className="text-gray-400"> · {item.due_hint}</span> : null}
                        </span>
                      </label>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
          {agents.length === 0 && (
            <div className="text-sm text-gray-400 py-2">No agents available.</div>
          )}
        </div>
      )}
    </div>
  );
}
