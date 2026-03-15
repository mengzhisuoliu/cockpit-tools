import { useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAccountStore } from '../stores/useAccountStore';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { useGitHubCopilotAccountStore } from '../stores/useGitHubCopilotAccountStore';
import { useWindsurfAccountStore } from '../stores/useWindsurfAccountStore';
import { useKiroAccountStore } from '../stores/useKiroAccountStore';
import { useCursorAccountStore } from '../stores/useCursorAccountStore';
import { useGeminiAccountStore } from '../stores/useGeminiAccountStore';
import { useCodebuddyAccountStore } from '../stores/useCodebuddyAccountStore';
import { useCodebuddyCnAccountStore } from '../stores/useCodebuddyCnAccountStore';
import { useQoderAccountStore } from '../stores/useQoderAccountStore';
import { useTraeAccountStore } from '../stores/useTraeAccountStore';

interface GeneralConfig {
  language: string;
  theme: string;
  auto_refresh_minutes: number;
  codex_auto_refresh_minutes: number;
  ghcp_auto_refresh_minutes: number;
  windsurf_auto_refresh_minutes: number;
  kiro_auto_refresh_minutes: number;
  cursor_auto_refresh_minutes: number;
  gemini_auto_refresh_minutes: number;
  codebuddy_auto_refresh_minutes: number;
  codebuddy_cn_auto_refresh_minutes: number;
  qoder_auto_refresh_minutes: number;
  trae_auto_refresh_minutes: number;
  auto_switch_enabled: boolean;
  close_behavior: string;
  opencode_app_path?: string;
  antigravity_app_path?: string;
  codex_app_path?: string;
  vscode_app_path?: string;
  windsurf_app_path?: string;
  kiro_app_path?: string;
  cursor_app_path?: string;
  codebuddy_app_path?: string;
  codebuddy_cn_app_path?: string;
  qoder_app_path?: string;
  trae_app_path?: string;
  opencode_sync_on_switch?: boolean;
  opencode_auth_overwrite_on_switch?: boolean;
  codex_launch_on_switch?: boolean;
  cursor_quota_alert_enabled?: boolean;
  cursor_quota_alert_threshold?: number;
  gemini_quota_alert_enabled?: boolean;
  gemini_quota_alert_threshold?: number;
}

const STARTUP_AUTO_REFRESH_SETUP_DELAY_MS = 2500;

export function useAutoRefresh() {
  const refreshAllQuotas = useAccountStore((state) => state.refreshAllQuotas);
  const syncCurrentFromClient = useAccountStore((state) => state.syncCurrentFromClient);
  const fetchAccounts = useAccountStore((state) => state.fetchAccounts);
  const fetchCurrentAccount = useAccountStore((state) => state.fetchCurrentAccount);

  const refreshAllCodexQuotas = useCodexAccountStore((state) => state.refreshAllQuotas);
  const refreshAllGhcpTokens = useGitHubCopilotAccountStore((state) => state.refreshAllTokens);
  const refreshAllWindsurfTokens = useWindsurfAccountStore((state) => state.refreshAllTokens);
  const refreshAllKiroTokens = useKiroAccountStore((state) => state.refreshAllTokens);
  const refreshAllCursorTokens = useCursorAccountStore((state) => state.refreshAllTokens);
  const refreshAllGeminiTokens = useGeminiAccountStore((state) => state.refreshAllTokens);
  const refreshAllCodebuddyTokens = useCodebuddyAccountStore((state) => state.refreshAllTokens);
  const refreshAllCodebuddyCnTokens = useCodebuddyCnAccountStore((state) => state.refreshAllTokens);
  const refreshAllQoderTokens = useQoderAccountStore((state) => state.refreshAllTokens);
  const refreshAllTraeTokens = useTraeAccountStore((state) => state.refreshAllTokens);

  const agIntervalRef = useRef<number | null>(null);
  const autoSwitchIntervalRef = useRef<number | null>(null);
  const codexIntervalRef = useRef<number | null>(null);
  const ghcpIntervalRef = useRef<number | null>(null);
  const windsurfIntervalRef = useRef<number | null>(null);
  const kiroIntervalRef = useRef<number | null>(null);
  const cursorIntervalRef = useRef<number | null>(null);
  const geminiIntervalRef = useRef<number | null>(null);
  const codebuddyIntervalRef = useRef<number | null>(null);
  const codebuddyCnIntervalRef = useRef<number | null>(null);
  const qoderIntervalRef = useRef<number | null>(null);
  const traeIntervalRef = useRef<number | null>(null);

  const agRefreshingRef = useRef(false);
  const codexRefreshingRef = useRef(false);
  const ghcpRefreshingRef = useRef(false);
  const windsurfRefreshingRef = useRef(false);
  const kiroRefreshingRef = useRef(false);
  const cursorRefreshingRef = useRef(false);
  const geminiRefreshingRef = useRef(false);
  const codebuddyRefreshingRef = useRef(false);
  const codebuddyCnRefreshingRef = useRef(false);
  const qoderRefreshingRef = useRef(false);
  const traeRefreshingRef = useRef(false);
  const autoSwitchRefreshingRef = useRef(false);

  const setupRunningRef = useRef(false);
  const setupPendingRef = useRef(false);
  const destroyedRef = useRef(false);

  const clearAllIntervals = useCallback(() => {
    if (agIntervalRef.current) {
      window.clearInterval(agIntervalRef.current);
      agIntervalRef.current = null;
    }
    if (codexIntervalRef.current) {
      window.clearInterval(codexIntervalRef.current);
      codexIntervalRef.current = null;
    }
    if (ghcpIntervalRef.current) {
      window.clearInterval(ghcpIntervalRef.current);
      ghcpIntervalRef.current = null;
    }
    if (autoSwitchIntervalRef.current) {
      window.clearInterval(autoSwitchIntervalRef.current);
      autoSwitchIntervalRef.current = null;
    }
    if (windsurfIntervalRef.current) {
      window.clearInterval(windsurfIntervalRef.current);
      windsurfIntervalRef.current = null;
    }
    if (kiroIntervalRef.current) {
      window.clearInterval(kiroIntervalRef.current);
      kiroIntervalRef.current = null;
    }
    if (cursorIntervalRef.current) {
      window.clearInterval(cursorIntervalRef.current);
      cursorIntervalRef.current = null;
    }
    if (geminiIntervalRef.current) {
      window.clearInterval(geminiIntervalRef.current);
      geminiIntervalRef.current = null;
    }
    if (codebuddyIntervalRef.current) {
      window.clearInterval(codebuddyIntervalRef.current);
      codebuddyIntervalRef.current = null;
    }
    if (codebuddyCnIntervalRef.current) {
      window.clearInterval(codebuddyCnIntervalRef.current);
      codebuddyCnIntervalRef.current = null;
    }
    if (qoderIntervalRef.current) {
      window.clearInterval(qoderIntervalRef.current);
      qoderIntervalRef.current = null;
    }
    if (traeIntervalRef.current) {
      window.clearInterval(traeIntervalRef.current);
      traeIntervalRef.current = null;
    }
  }, []);

  const setupAutoRefresh = useCallback(async () => {
    const setupStartedAt = performance.now();
    console.log('[StartupPerf][AutoRefresh] setupAutoRefresh start');

    if (destroyedRef.current) {
      console.log('[StartupPerf][AutoRefresh] setupAutoRefresh aborted: destroyed flag set');
      return;
    }

    if (setupRunningRef.current) {
      setupPendingRef.current = true;
      console.log('[StartupPerf][AutoRefresh] setupAutoRefresh skipped: previous run still active');
      return;
    }

    setupRunningRef.current = true;

    try {
      do {
        setupPendingRef.current = false;

        try {
          const configInvokeStartedAt = performance.now();
          const config = await invoke<GeneralConfig>('get_general_config');
          console.log(
            `[StartupPerf][AutoRefresh] get_general_config completed in ${(performance.now() - configInvokeStartedAt).toFixed(2)}ms`,
          );
          if (destroyedRef.current) {
            console.log('[StartupPerf][AutoRefresh] setupAutoRefresh aborted after config load: destroyed flag set');
            return;
          }

          // 检测配额重置任务状态及唤醒总开关
          const wakeupEnabled = localStorage.getItem('agtools.wakeup.enabled') === 'true';
          if (wakeupEnabled) {
            const tasksJson = localStorage.getItem('agtools.wakeup.tasks');
            if (tasksJson) {
              try {
                const tasks = JSON.parse(tasksJson);
                const hasActiveResetTask = Array.isArray(tasks) && tasks.some(
                  (task: unknown) => {
                    if (!task || typeof task !== 'object') {
                      return false;
                    }
                    const taskObj = task as {
                      enabled?: boolean;
                      schedule?: { wakeOnReset?: boolean };
                    };
                    return Boolean(taskObj.enabled && taskObj.schedule?.wakeOnReset);
                  },
                );

                // 如果有活跃的重置任务，且刷新间隔为禁用(-1)或大于2分钟，则强制修正为2分钟
                if (hasActiveResetTask && (config.auto_refresh_minutes === -1 || config.auto_refresh_minutes > 2)) {
                  console.log(`[AutoRefresh] 检测到活跃的配额重置任务，自动修正刷新间隔: ${config.auto_refresh_minutes} -> 2`);
                  const saveConfigStartedAt = performance.now();
                  await invoke('save_general_config', {
                    language: config.language,
                    theme: config.theme,
                    autoRefreshMinutes: 2,
                    codexAutoRefreshMinutes: config.codex_auto_refresh_minutes,
                    ghcpAutoRefreshMinutes: config.ghcp_auto_refresh_minutes,
                    windsurfAutoRefreshMinutes: config.windsurf_auto_refresh_minutes,
                    kiroAutoRefreshMinutes: config.kiro_auto_refresh_minutes,
                    cursorAutoRefreshMinutes: config.cursor_auto_refresh_minutes,
                    geminiAutoRefreshMinutes: config.gemini_auto_refresh_minutes,
                    codebuddyAutoRefreshMinutes: config.codebuddy_auto_refresh_minutes,
                    codebuddyCnAutoRefreshMinutes: config.codebuddy_cn_auto_refresh_minutes,
                    qoderAutoRefreshMinutes: config.qoder_auto_refresh_minutes,
                    traeAutoRefreshMinutes: config.trae_auto_refresh_minutes,
                    closeBehavior: config.close_behavior || 'ask',
                    opencodeAppPath: config.opencode_app_path ?? '',
                    antigravityAppPath: config.antigravity_app_path ?? '',
                    codexAppPath: config.codex_app_path ?? '',
                    vscodeAppPath: config.vscode_app_path ?? '',
                    windsurfAppPath: config.windsurf_app_path ?? '',
                    kiroAppPath: config.kiro_app_path ?? '',
                    cursorAppPath: config.cursor_app_path ?? '',
                    codebuddyAppPath: config.codebuddy_app_path ?? '',
                    codebuddyCnAppPath: config.codebuddy_cn_app_path ?? '',
                    qoderAppPath: config.qoder_app_path ?? '',
                    traeAppPath: config.trae_app_path ?? '',
                    opencodeSyncOnSwitch: config.opencode_sync_on_switch ?? true,
                    opencodeAuthOverwriteOnSwitch:
                      config.opencode_auth_overwrite_on_switch ?? true,
                    codexLaunchOnSwitch: config.codex_launch_on_switch ?? true,
                    cursorQuotaAlertEnabled: config.cursor_quota_alert_enabled ?? false,
                    cursorQuotaAlertThreshold: config.cursor_quota_alert_threshold ?? 20,
                    geminiQuotaAlertEnabled: config.gemini_quota_alert_enabled ?? false,
                    geminiQuotaAlertThreshold: config.gemini_quota_alert_threshold ?? 20,
                  });
                  console.log(
                    `[StartupPerf][AutoRefresh] save_general_config completed in ${(performance.now() - saveConfigStartedAt).toFixed(2)}ms`,
                  );
                  config.auto_refresh_minutes = 2;
                }
              } catch (e) {
                console.error('[AutoRefresh] 解析任务列表失败:', e);
              }
            }
          }

          if (destroyedRef.current) {
            console.log('[StartupPerf][AutoRefresh] setupAutoRefresh aborted before interval setup: destroyed flag set');
            return;
          }

          clearAllIntervals();

          if (config.auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Antigravity 已启用: 每 ${config.auto_refresh_minutes} 分钟`);
            const agMs = config.auto_refresh_minutes * 60 * 1000;

            agIntervalRef.current = window.setInterval(async () => {
              if (agRefreshingRef.current) {
                return;
              }
              agRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发定时配额刷新...');
                await syncCurrentFromClient();
                await refreshAllQuotas();
              } catch (e) {
                console.error('[AutoRefresh] 刷新失败:', e);
              } finally {
                agRefreshingRef.current = false;
              }
            }, agMs);
          } else {
            console.log('[AutoRefresh] Antigravity 已禁用');
          }

          if (config.codex_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Codex 已启用: 每 ${config.codex_auto_refresh_minutes} 分钟`);
            const codexMs = config.codex_auto_refresh_minutes * 60 * 1000;

            codexIntervalRef.current = window.setInterval(async () => {
              if (codexRefreshingRef.current) {
                return;
              }
              codexRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 Codex 配额刷新...');
                await refreshAllCodexQuotas();
              } catch (e) {
                console.error('[AutoRefresh] Codex 刷新失败:', e);
              } finally {
                codexRefreshingRef.current = false;
              }
            }, codexMs);
          } else {
            console.log('[AutoRefresh] Codex 已禁用');
          }

          if (config.ghcp_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] GitHub Copilot 已启用: 每 ${config.ghcp_auto_refresh_minutes} 分钟`);
            const ghcpMs = config.ghcp_auto_refresh_minutes * 60 * 1000;

            ghcpIntervalRef.current = window.setInterval(async () => {
              if (ghcpRefreshingRef.current) {
                return;
              }
              ghcpRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 GitHub Copilot Token 刷新...');
                await refreshAllGhcpTokens();
              } catch (e) {
                console.error('[AutoRefresh] GitHub Copilot 刷新失败:', e);
              } finally {
                ghcpRefreshingRef.current = false;
              }
            }, ghcpMs);
          } else {
            console.log('[AutoRefresh] GitHub Copilot 已禁用');
          }

          if (config.windsurf_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Windsurf 已启用: 每 ${config.windsurf_auto_refresh_minutes} 分钟`);
            const windsurfMs = config.windsurf_auto_refresh_minutes * 60 * 1000;

            windsurfIntervalRef.current = window.setInterval(async () => {
              if (windsurfRefreshingRef.current) {
                return;
              }
              windsurfRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 Windsurf 配额刷新...');
                await refreshAllWindsurfTokens();
              } catch (e) {
                console.error('[AutoRefresh] Windsurf 刷新失败:', e);
              } finally {
                windsurfRefreshingRef.current = false;
              }
            }, windsurfMs);
          } else {
            console.log('[AutoRefresh] Windsurf 已禁用');
          }

          if (config.kiro_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Kiro 已启用: 每 ${config.kiro_auto_refresh_minutes} 分钟`);
            const kiroMs = config.kiro_auto_refresh_minutes * 60 * 1000;

            kiroIntervalRef.current = window.setInterval(async () => {
              if (kiroRefreshingRef.current) {
                return;
              }
              kiroRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 Kiro 配额刷新...');
                await refreshAllKiroTokens();
              } catch (e) {
                console.error('[AutoRefresh] Kiro 刷新失败:', e);
              } finally {
                kiroRefreshingRef.current = false;
              }
            }, kiroMs);
          } else {
            console.log('[AutoRefresh] Kiro 已禁用');
          }

          if (config.cursor_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Cursor 已启用: 每 ${config.cursor_auto_refresh_minutes} 分钟`);
            const cursorMs = config.cursor_auto_refresh_minutes * 60 * 1000;

            cursorIntervalRef.current = window.setInterval(async () => {
              if (cursorRefreshingRef.current) {
                return;
              }
              cursorRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 Cursor 配额刷新...');
                await refreshAllCursorTokens();
              } catch (e) {
                console.error('[AutoRefresh] Cursor 刷新失败:', e);
              } finally {
                cursorRefreshingRef.current = false;
              }
            }, cursorMs);
          } else {
            console.log('[AutoRefresh] Cursor 已禁用');
          }

          if (config.gemini_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Gemini 已启用: 每 ${config.gemini_auto_refresh_minutes} 分钟`);
            const geminiMs = config.gemini_auto_refresh_minutes * 60 * 1000;

            geminiIntervalRef.current = window.setInterval(async () => {
              if (geminiRefreshingRef.current) {
                return;
              }
              geminiRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 Gemini 配额刷新...');
                await refreshAllGeminiTokens();
              } catch (e) {
                console.error('[AutoRefresh] Gemini 刷新失败:', e);
              } finally {
                geminiRefreshingRef.current = false;
              }
            }, geminiMs);
          } else {
            console.log('[AutoRefresh] Gemini 已禁用');
          }

          if (config.codebuddy_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] CodeBuddy 已启用: 每 ${config.codebuddy_auto_refresh_minutes} 分钟`);
            const codebuddyMs = config.codebuddy_auto_refresh_minutes * 60 * 1000;

            codebuddyIntervalRef.current = window.setInterval(async () => {
              if (codebuddyRefreshingRef.current) {
                return;
              }
              codebuddyRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 CodeBuddy 配额刷新...');
                await refreshAllCodebuddyTokens();
              } catch (e) {
                console.error('[AutoRefresh] CodeBuddy 刷新失败:', e);
              } finally {
                codebuddyRefreshingRef.current = false;
              }
            }, codebuddyMs);
          } else {
            console.log('[AutoRefresh] CodeBuddy 已禁用');
          }

          if (config.codebuddy_cn_auto_refresh_minutes > 0) {
            console.log(
              `[AutoRefresh] CodeBuddy CN 已启用: 每 ${config.codebuddy_cn_auto_refresh_minutes} 分钟`,
            );
            const codebuddyCnMs = config.codebuddy_cn_auto_refresh_minutes * 60 * 1000;

            codebuddyCnIntervalRef.current = window.setInterval(async () => {
              if (codebuddyCnRefreshingRef.current) {
                return;
              }
              codebuddyCnRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 CodeBuddy CN 配额刷新...');
                await refreshAllCodebuddyCnTokens();
              } catch (e) {
                console.error('[AutoRefresh] CodeBuddy CN 刷新失败:', e);
              } finally {
                codebuddyCnRefreshingRef.current = false;
              }
            }, codebuddyCnMs);
          } else {
            console.log('[AutoRefresh] CodeBuddy CN 已禁用');
          }

          if (config.qoder_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Qoder 已启用: 每 ${config.qoder_auto_refresh_minutes} 分钟`);
            const qoderMs = config.qoder_auto_refresh_minutes * 60 * 1000;

            qoderIntervalRef.current = window.setInterval(async () => {
              if (qoderRefreshingRef.current) {
                return;
              }
              qoderRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 Qoder 配额刷新...');
                await refreshAllQoderTokens();
              } catch (e) {
                console.error('[AutoRefresh] Qoder 刷新失败:', e);
              } finally {
                qoderRefreshingRef.current = false;
              }
            }, qoderMs);
          } else {
            console.log('[AutoRefresh] Qoder 已禁用');
          }

          if (config.trae_auto_refresh_minutes > 0) {
            console.log(`[AutoRefresh] Trae 已启用: 每 ${config.trae_auto_refresh_minutes} 分钟`);
            const traeMs = config.trae_auto_refresh_minutes * 60 * 1000;

            traeIntervalRef.current = window.setInterval(async () => {
              if (traeRefreshingRef.current) {
                return;
              }
              traeRefreshingRef.current = true;

              try {
                console.log('[AutoRefresh] 触发 Trae 配额刷新...');
                await refreshAllTraeTokens();
              } catch (e) {
                console.error('[AutoRefresh] Trae 刷新失败:', e);
              } finally {
                traeRefreshingRef.current = false;
              }
            }, traeMs);
          } else {
            console.log('[AutoRefresh] Trae 已禁用');
          }

          // 自动切号开启时，额外每 60 秒刷新当前账号（不影响原有配额自动刷新规则）
          if (config.auto_switch_enabled) {
            console.log('[AutoRefresh] 自动切号已启用: 每 60 秒刷新当前账号');
            autoSwitchIntervalRef.current = window.setInterval(async () => {
              if (autoSwitchRefreshingRef.current) {
                return;
              }
              autoSwitchRefreshingRef.current = true;

              try {
                await syncCurrentFromClient();
                await invoke('refresh_current_quota');
                await fetchAccounts();
                await fetchCurrentAccount();
              } catch (e) {
                console.error('[AutoRefresh] 自动切号-当前账号刷新失败:', e);
              } finally {
                autoSwitchRefreshingRef.current = false;
              }
            }, 60 * 1000);
          } else {
            console.log('[AutoRefresh] 自动切号未启用，跳过 60 秒当前账号刷新');
          }

          const enabledPlatforms = [
            config.auto_refresh_minutes > 0 ? `antigravity=${config.auto_refresh_minutes}` : null,
            config.codex_auto_refresh_minutes > 0 ? `codex=${config.codex_auto_refresh_minutes}` : null,
            config.ghcp_auto_refresh_minutes > 0 ? `ghcp=${config.ghcp_auto_refresh_minutes}` : null,
            config.windsurf_auto_refresh_minutes > 0 ? `windsurf=${config.windsurf_auto_refresh_minutes}` : null,
            config.kiro_auto_refresh_minutes > 0 ? `kiro=${config.kiro_auto_refresh_minutes}` : null,
            config.cursor_auto_refresh_minutes > 0 ? `cursor=${config.cursor_auto_refresh_minutes}` : null,
            config.gemini_auto_refresh_minutes > 0 ? `gemini=${config.gemini_auto_refresh_minutes}` : null,
            config.codebuddy_auto_refresh_minutes > 0 ? `codebuddy=${config.codebuddy_auto_refresh_minutes}` : null,
            config.codebuddy_cn_auto_refresh_minutes > 0
              ? `codebuddy_cn=${config.codebuddy_cn_auto_refresh_minutes}`
              : null,
            config.qoder_auto_refresh_minutes > 0 ? `qoder=${config.qoder_auto_refresh_minutes}` : null,
            config.trae_auto_refresh_minutes > 0 ? `trae=${config.trae_auto_refresh_minutes}` : null,
            config.auto_switch_enabled ? 'auto_switch=60s' : null,
          ].filter(Boolean).join(', ');
          console.log(
            `[StartupPerf][AutoRefresh] setupAutoRefresh completed in ${(performance.now() - setupStartedAt).toFixed(2)}ms; enabled=${enabledPlatforms || 'none'}`,
          );
        } catch (err) {
          console.error('[AutoRefresh] 加载配置失败:', err);
          console.error(
            `[StartupPerf][AutoRefresh] setupAutoRefresh failed after ${(performance.now() - setupStartedAt).toFixed(2)}ms:`,
            err,
          );
        }
      } while (setupPendingRef.current && !destroyedRef.current);
    } finally {
      setupRunningRef.current = false;
      console.log(
        `[StartupPerf][AutoRefresh] setupAutoRefresh exit after ${(performance.now() - setupStartedAt).toFixed(2)}ms`,
      );
    }
  }, [
    clearAllIntervals,
    fetchAccounts,
    fetchCurrentAccount,
    refreshAllCodexQuotas,
    refreshAllCursorTokens,
    refreshAllGeminiTokens,
    refreshAllGhcpTokens,
    refreshAllKiroTokens,
    refreshAllCodebuddyTokens,
    refreshAllCodebuddyCnTokens,
    refreshAllQoderTokens,
    refreshAllTraeTokens,
    refreshAllQuotas,
    refreshAllWindsurfTokens,
    syncCurrentFromClient,
  ]);

  useEffect(() => {
    destroyedRef.current = false;
    let startupTimer = window.setTimeout(() => {
      startupTimer = 0;
      console.log(
        `[StartupPerf][AutoRefresh] deferred startup setup triggered after ${STARTUP_AUTO_REFRESH_SETUP_DELAY_MS}ms`,
      );
      void setupAutoRefresh();
    }, STARTUP_AUTO_REFRESH_SETUP_DELAY_MS);

    const handleConfigUpdate = () => {
      if (startupTimer) {
        window.clearTimeout(startupTimer);
        startupTimer = 0;
      }
      console.log('[AutoRefresh] 检测到配置变更，重新设置定时器');
      void setupAutoRefresh();
    };

    window.addEventListener('config-updated', handleConfigUpdate);

    return () => {
      destroyedRef.current = true;
      setupPendingRef.current = false;
      if (startupTimer) {
        window.clearTimeout(startupTimer);
      }
      clearAllIntervals();
      window.removeEventListener('config-updated', handleConfigUpdate);
    };
  }, [clearAllIntervals, setupAutoRefresh]);
}
