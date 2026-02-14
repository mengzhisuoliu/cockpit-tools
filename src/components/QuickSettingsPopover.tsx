import { useState, useEffect, useRef, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { Settings, RefreshCw, FolderOpen, Zap, X } from 'lucide-react';
import './QuickSettingsPopover.css';

/** GeneralConfig from backend */
interface GeneralConfig {
  language: string;
  theme: string;
  auto_refresh_minutes: number;
  codex_auto_refresh_minutes: number;
  ghcp_auto_refresh_minutes: number;
  windsurf_auto_refresh_minutes: number;
  close_behavior: string;
  opencode_app_path: string;
  antigravity_app_path: string;
  codex_app_path: string;
  vscode_app_path: string;
  windsurf_app_path: string;
  opencode_sync_on_switch: boolean;
  auto_switch_enabled: boolean;
  auto_switch_threshold: number;
  quota_alert_enabled: boolean;
  quota_alert_threshold: number;
  codex_quota_alert_enabled: boolean;
  codex_quota_alert_threshold: number;
  ghcp_quota_alert_enabled: boolean;
  ghcp_quota_alert_threshold: number;
  windsurf_quota_alert_enabled: boolean;
  windsurf_quota_alert_threshold: number;
}

export type QuickSettingsType = 'antigravity' | 'codex' | 'github_copilot' | 'windsurf';

type QuotaAlertEnabledKey =
  | 'quota_alert_enabled'
  | 'codex_quota_alert_enabled'
  | 'ghcp_quota_alert_enabled'
  | 'windsurf_quota_alert_enabled';
type QuotaAlertThresholdKey =
  | 'quota_alert_threshold'
  | 'codex_quota_alert_threshold'
  | 'ghcp_quota_alert_threshold'
  | 'windsurf_quota_alert_threshold';

interface QuickSettingsPopoverProps {
  type: QuickSettingsType;
}

export function QuickSettingsPopover({ type }: QuickSettingsPopoverProps) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [config, setConfig] = useState<GeneralConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [pathDetecting, setPathDetecting] = useState(false);
  const [refreshEditing, setRefreshEditing] = useState(false);
  const [thresholdEditing, setThresholdEditing] = useState(false);
  const [quotaAlertThresholdEditing, setQuotaAlertThresholdEditing] = useState(false);
  const [customRefresh, setCustomRefresh] = useState('');
  const [customThreshold, setCustomThreshold] = useState('');
  const [quotaAlertCustomThreshold, setQuotaAlertCustomThreshold] = useState('');
  const modalRef = useRef<HTMLDivElement>(null);
  const refreshPresets = ['-1', '2', '5', '10', '15'];
  const thresholdPresets = ['0', '20', '40', '60'];

  // Load config when modal opens
  useEffect(() => {
    if (isOpen) {
      loadConfig();
    }
  }, [isOpen]);

  // Close on Escape
  useEffect(() => {
    if (!isOpen) return;

    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setIsOpen(false);
    };

    document.addEventListener('keydown', handleEsc);
    return () => {
      document.removeEventListener('keydown', handleEsc);
    };
  }, [isOpen]);

  const loadConfig = async () => {
    try {
      const cfg = await invoke<GeneralConfig>('get_general_config');
      setConfig(cfg);
      // 非预设值通过下拉中的动态选项展示，不默认进入输入态
      setRefreshEditing(false);
      setThresholdEditing(false);
      setQuotaAlertThresholdEditing(false);
      setCustomRefresh('');
      setCustomThreshold('');
      setQuotaAlertCustomThreshold('');
    } catch (err) {
      console.error('Failed to load config:', err);
    }
  };

  const getRefreshKeyForType = (t: QuickSettingsType): keyof GeneralConfig => {
    switch (t) {
      case 'antigravity': return 'auto_refresh_minutes';
      case 'codex': return 'codex_auto_refresh_minutes';
      case 'github_copilot': return 'ghcp_auto_refresh_minutes';
      case 'windsurf': return 'windsurf_auto_refresh_minutes';
    }
  };

  const saveConfig = useCallback(
    async (updates: Partial<GeneralConfig>) => {
      if (!config || saving) return;
      const merged = { ...config, ...updates };
      setConfig(merged);
      setSaving(true);
      try {
        await invoke('save_general_config', {
          language: merged.language,
          theme: merged.theme,
          autoRefreshMinutes: merged.auto_refresh_minutes,
          codexAutoRefreshMinutes: merged.codex_auto_refresh_minutes,
          ghcpAutoRefreshMinutes: merged.ghcp_auto_refresh_minutes,
          windsurfAutoRefreshMinutes: merged.windsurf_auto_refresh_minutes,
          closeBehavior: merged.close_behavior,
          opencodeAppPath: merged.opencode_app_path,
          antigravityAppPath: merged.antigravity_app_path,
          codexAppPath: merged.codex_app_path,
          vscodeAppPath: merged.vscode_app_path,
          windsurfAppPath: merged.windsurf_app_path,
          opencodeSyncOnSwitch: merged.opencode_sync_on_switch,
          autoSwitchEnabled: merged.auto_switch_enabled,
          autoSwitchThreshold: merged.auto_switch_threshold,
          quotaAlertEnabled: merged.quota_alert_enabled,
          quotaAlertThreshold: merged.quota_alert_threshold,
          codexQuotaAlertEnabled: merged.codex_quota_alert_enabled,
          codexQuotaAlertThreshold: merged.codex_quota_alert_threshold,
          ghcpQuotaAlertEnabled: merged.ghcp_quota_alert_enabled,
          ghcpQuotaAlertThreshold: merged.ghcp_quota_alert_threshold,
          windsurfQuotaAlertEnabled: merged.windsurf_quota_alert_enabled,
          windsurfQuotaAlertThreshold: merged.windsurf_quota_alert_threshold,
        });
        window.dispatchEvent(new Event('config-updated'));
      } catch (err) {
        console.error('Failed to save config:', err);
      } finally {
        setSaving(false);
      }
    },
    [config, saving]
  );

  const handlePickAppPath = async (target: 'antigravity' | 'codex' | 'vscode' | 'windsurf') => {
    try {
      const selected = await open({ multiple: false, directory: false });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path || !config) return;

      const key =
        target === 'antigravity'
          ? 'antigravity_app_path'
          : target === 'codex'
            ? 'codex_app_path'
            : target === 'vscode'
              ? 'vscode_app_path'
              : 'windsurf_app_path';

      saveConfig({ [key]: path });
    } catch (err) {
      console.error('Failed to pick path:', err);
    }
  };

  const handleResetAppPath = async (target: 'antigravity' | 'codex' | 'vscode' | 'windsurf') => {
    if (pathDetecting) return;
    setPathDetecting(true);
    try {
      const detected = await invoke<string | null>('detect_app_path', { app: target, force: true });
      const path = detected || '';
      const key =
        target === 'antigravity'
          ? 'antigravity_app_path'
          : target === 'codex'
            ? 'codex_app_path'
            : target === 'vscode'
              ? 'vscode_app_path'
              : 'windsurf_app_path';
      saveConfig({ [key]: path });
    } catch (err) {
      console.error('Failed to reset path:', err);
    } finally {
      setPathDetecting(false);
    }
  };

  const getTitle = () => {
    switch (type) {
      case 'antigravity':
        return t('quickSettings.antigravity.title', 'Antigravity 设置');
      case 'codex':
        return t('quickSettings.codex.title', 'Codex 设置');
      case 'github_copilot':
        return t('quickSettings.githubCopilot.title', 'GitHub Copilot 设置');
      case 'windsurf':
        return t('quickSettings.windsurf.title', 'Windsurf 设置');
    }
  };

  const getRefreshKey = (): keyof GeneralConfig => {
    return getRefreshKeyForType(type);
  };

  const getQuotaAlertEnabledKeyForType = (t: QuickSettingsType): QuotaAlertEnabledKey => {
    switch (t) {
      case 'codex':
        return 'codex_quota_alert_enabled';
      case 'github_copilot':
        return 'ghcp_quota_alert_enabled';
      case 'windsurf':
        return 'windsurf_quota_alert_enabled';
      default:
        return 'quota_alert_enabled';
    }
  };

  const getQuotaAlertThresholdKeyForType = (t: QuickSettingsType): QuotaAlertThresholdKey => {
    switch (t) {
      case 'codex':
        return 'codex_quota_alert_threshold';
      case 'github_copilot':
        return 'ghcp_quota_alert_threshold';
      case 'windsurf':
        return 'windsurf_quota_alert_threshold';
      default:
        return 'quota_alert_threshold';
    }
  };

  const getRefreshLabel = () => {
    switch (type) {
      case 'antigravity':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'codex':
        return t('quickSettings.codexRefreshInterval', '配额自动刷新');
      case 'github_copilot':
        return t('quickSettings.ghcpRefreshInterval', '配额自动刷新');
      case 'windsurf':
        return t('quickSettings.windsurfRefreshInterval', '配额自动刷新');
    }
  };

  const getAppPath = (): string => {
    if (!config) return '';
    switch (type) {
      case 'antigravity':
        return config.antigravity_app_path;
      case 'codex':
        return config.codex_app_path;
      case 'github_copilot':
        return config.vscode_app_path;
      case 'windsurf':
        return config.windsurf_app_path;
    }
  };

  const getAppPathLabel = () => {
    switch (type) {
      case 'antigravity':
        return t('quickSettings.antigravity.appPath', '启动路径');
      case 'codex':
        return t('quickSettings.codex.appPath', '启动路径');
      case 'github_copilot':
        return t('quickSettings.githubCopilot.appPath', 'VS Code 路径');
      case 'windsurf':
        return t('quickSettings.windsurf.appPath', 'Windsurf 路径');
    }
  };

  const getAppTarget = (): 'antigravity' | 'codex' | 'vscode' | 'windsurf' => {
    switch (type) {
      case 'antigravity':
        return 'antigravity';
      case 'codex':
        return 'codex';
      case 'github_copilot':
        return 'vscode';
      case 'windsurf':
        return 'windsurf';
    }
  };

  const refreshValue = config ? (config[getRefreshKey()] as number) : 10;
  const isPreset = refreshPresets.includes(String(refreshValue));
  const showRefreshInput = refreshEditing;

  const isThresholdPreset = config ? thresholdPresets.includes(String(config.auto_switch_threshold)) : true;
  const showThresholdInput = thresholdEditing;
  const quotaAlertEnabledKey = getQuotaAlertEnabledKeyForType(type);
  const quotaAlertThresholdKey = getQuotaAlertThresholdKeyForType(type);
  const quotaAlertEnabledValue = config ? Boolean(config[quotaAlertEnabledKey]) : false;
  const quotaAlertThresholdValue = config ? Number(config[quotaAlertThresholdKey]) : 20;
  const isQuotaAlertThresholdPreset = thresholdPresets.includes(String(quotaAlertThresholdValue));
  const showQuotaAlertThresholdInput = quotaAlertThresholdEditing;

  const handleRefreshSelectChange = (val: string) => {
    if (val === 'custom') {
      setCustomRefresh(String(refreshValue > 0 ? refreshValue : 1));
      setRefreshEditing(true);
    } else {
      setCustomRefresh('');
      setRefreshEditing(false);
      saveConfig({ [getRefreshKey()]: parseInt(val, 10) });
    }
  };

  const handleCustomRefreshApply = () => {
    const parsed = parseInt(customRefresh, 10);
    if (!isNaN(parsed) && parsed >= 1) {
      saveConfig({ [getRefreshKey()]: parsed });
      setCustomRefresh('');
      setRefreshEditing(false);
      return;
    }
    setCustomRefresh('');
    setRefreshEditing(false);
  };

  const handleThresholdSelectChange = (val: string) => {
    if (val === 'custom') {
      setCustomThreshold(String(config?.auto_switch_threshold ?? 20));
      setThresholdEditing(true);
    } else {
      setCustomThreshold('');
      setThresholdEditing(false);
      saveConfig({ auto_switch_threshold: parseInt(val, 10) });
    }
  };

  const handleCustomThresholdApply = () => {
    const parsed = parseInt(customThreshold, 10);
    if (!isNaN(parsed) && parsed >= 0 && parsed <= 100) {
      saveConfig({ auto_switch_threshold: parsed });
      setCustomThreshold('');
      setThresholdEditing(false);
      return;
    }
    setCustomThreshold('');
    setThresholdEditing(false);
  };

  const handleQuotaAlertThresholdSelectChange = (val: string) => {
    if (val === 'custom') {
      setQuotaAlertCustomThreshold(String(quotaAlertThresholdValue));
      setQuotaAlertThresholdEditing(true);
    } else {
      setQuotaAlertCustomThreshold('');
      setQuotaAlertThresholdEditing(false);
      saveConfig({ [quotaAlertThresholdKey]: parseInt(val, 10) } as Partial<GeneralConfig>);
    }
  };

  const handleQuotaAlertCustomThresholdApply = () => {
    const parsed = parseInt(quotaAlertCustomThreshold, 10);
    if (!isNaN(parsed) && parsed >= 0 && parsed <= 100) {
      saveConfig({ [quotaAlertThresholdKey]: parsed } as Partial<GeneralConfig>);
      setQuotaAlertCustomThreshold('');
      setQuotaAlertThresholdEditing(false);
      return;
    }
    setQuotaAlertCustomThreshold('');
    setQuotaAlertThresholdEditing(false);
  };

  const overlayContent = isOpen ? (
    <div className="qs-overlay" onClick={(e) => { if (e.target === e.currentTarget) setIsOpen(false); }}>
      <div className="qs-modal" ref={modalRef}>
        <div className="qs-header">
          <span className="qs-title">{getTitle()}</span>
          <button className="qs-close" onClick={() => setIsOpen(false)} aria-label={t('common.close')}>
            <X size={16} />
          </button>
        </div>

        {config && (
          <div className="qs-body">
            {/* ─── Refresh Interval ─── */}
            <div className="qs-section">
              <div className="qs-section-header">
                <RefreshCw size={15} />
                <span>{getRefreshLabel()}</span>
              </div>
              <div className="qs-field-group">
                {showRefreshInput ? (
                  <div className="qs-inline-input">
                    <input
                      type="number"
                      min={1}
                      max={999}
                      className="qs-select qs-select--input-mode qs-select--with-unit"
                      value={customRefresh}
                      placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                      onChange={(e) => setCustomRefresh(e.target.value.replace(/[^\d]/g, ''))}
                      onBlur={handleCustomRefreshApply}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.preventDefault();
                          handleCustomRefreshApply();
                        }
                      }}
                    />
                    <span className="qs-input-unit">{t('settings.general.minutes')}</span>
                  </div>
                ) : (
                  <select
                    className="qs-select"
                    value={String(refreshValue)}
                    onChange={(e) => handleRefreshSelectChange(e.target.value)}
                  >
                    {!isPreset && (
                      <option value={String(refreshValue)}>
                        {refreshValue} {t('settings.general.minutes')}
                      </option>
                    )}
                    <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                    <option value="2">2 {t('settings.general.minutes')}</option>
                    <option value="5">5 {t('settings.general.minutes')}</option>
                    <option value="10">10 {t('settings.general.minutes')}</option>
                    <option value="15">15 {t('settings.general.minutes')}</option>
                    <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                  </select>
                )}
              </div>
            </div>

            {/* ─── App Path ─── */}
            <div className="qs-section">
              <div className="qs-section-header">
                <FolderOpen size={15} />
                <span>{getAppPathLabel()}</span>
              </div>
              <div className="qs-path-control">
                <input
                  type="text"
                  className="qs-path-input"
                  value={getAppPath()}
                  placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                  onChange={(e) => {
                    const key =
                      type === 'antigravity'
                        ? 'antigravity_app_path'
                        : type === 'codex'
                          ? 'codex_app_path'
                          : type === 'github_copilot'
                            ? 'vscode_app_path'
                            : 'windsurf_app_path';
                    saveConfig({ [key]: e.target.value });
                  }}
                />
                <div className="qs-path-actions">
                  <button
                    className="qs-btn"
                    onClick={() => handlePickAppPath(getAppTarget())}
                    disabled={pathDetecting}
                    title={t('settings.general.codexPathSelect', '选择')}
                  >
                    {t('settings.general.codexPathSelect', '选择')}
                  </button>
                  <button
                    className="qs-btn"
                    onClick={() => handleResetAppPath(getAppTarget())}
                    disabled={pathDetecting}
                    title={
                      pathDetecting
                        ? t('common.loading', '加载中...')
                        : t('settings.general.codexPathReset', '恢复默认')
                    }
                  >
                    <RefreshCw size={12} className={pathDetecting ? 'spin' : undefined} />
                  </button>
                </div>
              </div>
            </div>

            {/* ─── Codex: opencode sync ─── */}
            {type === 'codex' && (
              <div className="qs-section">
                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>{t('settings.general.opencodeRestart', '切换时同步 OpenCode')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.opencode_sync_on_switch}
                        onChange={(e) => saveConfig({ opencode_sync_on_switch: e.target.checked })}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>
              </div>
            )}

            {/* ─── Antigravity: Auto-switch ─── */}
            {type === 'antigravity' && (
              <div className="qs-section qs-section--highlight">
                <div className="qs-section-header">
                  <Zap size={15} />
                  <span>{t('quickSettings.autoSwitch.title', '自动切号')}</span>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <span>{t('quickSettings.autoSwitch.enable', '启用自动切号')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.auto_switch_enabled}
                        onChange={(e) => saveConfig({ auto_switch_enabled: e.target.checked })}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                {config.auto_switch_enabled && (
                  <div className="qs-field-group" style={{ animation: 'qsFadeUp 0.2s ease both' }}>
                    <div className="qs-row">
                      <div className="qs-row-label">
                        <span>{t('quickSettings.autoSwitch.threshold', '切号阈值')}</span>
                      </div>
                      <div className="qs-row-control">
                        {showThresholdInput ? (
                          <div className="qs-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="qs-select qs-select--input-mode qs-select--with-unit"
                              value={customThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setCustomThreshold(e.target.value.replace(/[^\d]/g, ''))}
                              onBlur={handleCustomThresholdApply}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  handleCustomThresholdApply();
                                }
                              }}
                            />
                            <span className="qs-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="qs-select"
                            value={String(config.auto_switch_threshold)}
                            onChange={(e) => handleThresholdSelectChange(e.target.value)}
                          >
                            {!isThresholdPreset && (
                              <option value={String(config.auto_switch_threshold)}>
                                {config.auto_switch_threshold}%
                              </option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  </div>
                )}

                <div className="qs-hint">
                  {t(
                    'quickSettings.autoSwitch.hint',
                    '当任意模型配额低于阈值时，自动切换到配额最高的账号。'
                  )}
                </div>

                <div className="qs-row" style={{ marginTop: 10 }}>
                  <div className="qs-row-label">
                    <span>{t('quickSettings.quotaAlert.enable', '超额预警')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={quotaAlertEnabledValue}
                        onChange={(e) =>
                          saveConfig({ [quotaAlertEnabledKey]: e.target.checked } as Partial<GeneralConfig>)
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                {quotaAlertEnabledValue && (
                  <div className="qs-field-group" style={{ animation: 'qsFadeUp 0.2s ease both' }}>
                    <div className="qs-row">
                      <div className="qs-row-label">
                        <span>{t('quickSettings.quotaAlert.threshold', '预警阈值')}</span>
                      </div>
                      <div className="qs-row-control">
                        {showQuotaAlertThresholdInput ? (
                          <div className="qs-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="qs-select qs-select--input-mode qs-select--with-unit"
                              value={quotaAlertCustomThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setQuotaAlertCustomThreshold(e.target.value.replace(/[^\d]/g, ''))}
                              onBlur={handleQuotaAlertCustomThresholdApply}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  handleQuotaAlertCustomThresholdApply();
                                }
                              }}
                            />
                            <span className="qs-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="qs-select"
                            value={String(quotaAlertThresholdValue)}
                            onChange={(e) => handleQuotaAlertThresholdSelectChange(e.target.value)}
                          >
                            {!isQuotaAlertThresholdPreset && (
                              <option value={String(quotaAlertThresholdValue)}>
                                {quotaAlertThresholdValue}%
                              </option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                    <div className="qs-hint" style={{ marginTop: 6 }}>
                      {t(
                        'quickSettings.quotaAlert.hint',
                        '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。'
                      )}
                    </div>
                  </div>
                )}
              </div>
            )}

            {type !== 'antigravity' && (
              <div className="qs-section qs-section--highlight">
                <div className="qs-section-header">
                  <Zap size={15} />
                  <span>{t('quickSettings.quotaAlert.enable', '超额预警')}</span>
                </div>
                <div className="qs-row">
                  <div className="qs-row-label">
                    <span>{t('quickSettings.quotaAlert.enable', '超额预警')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={quotaAlertEnabledValue}
                        onChange={(e) =>
                          saveConfig({ [quotaAlertEnabledKey]: e.target.checked } as Partial<GeneralConfig>)
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                {quotaAlertEnabledValue && (
                  <div className="qs-field-group" style={{ animation: 'qsFadeUp 0.2s ease both' }}>
                    <div className="qs-row">
                      <div className="qs-row-label">
                        <span>{t('quickSettings.quotaAlert.threshold', '预警阈值')}</span>
                      </div>
                      <div className="qs-row-control">
                        {showQuotaAlertThresholdInput ? (
                          <div className="qs-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="qs-select qs-select--input-mode qs-select--with-unit"
                              value={quotaAlertCustomThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setQuotaAlertCustomThreshold(e.target.value.replace(/[^\d]/g, ''))}
                              onBlur={handleQuotaAlertCustomThresholdApply}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  handleQuotaAlertCustomThresholdApply();
                                }
                              }}
                            />
                            <span className="qs-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="qs-select"
                            value={String(quotaAlertThresholdValue)}
                            onChange={(e) => handleQuotaAlertThresholdSelectChange(e.target.value)}
                          >
                            {!isQuotaAlertThresholdPreset && (
                              <option value={String(quotaAlertThresholdValue)}>
                                {quotaAlertThresholdValue}%
                              </option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                    <div className="qs-hint" style={{ marginTop: 6 }}>
                      {t(
                        'quickSettings.quotaAlert.hint',
                        '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。'
                      )}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  ) : null;

  return (
    <div className="quick-settings-wrapper">
      <button
        className={`btn btn-secondary icon-only ${isOpen ? 'active' : ''}`}
        onClick={() => setIsOpen(!isOpen)}
        title={getTitle()}
        aria-label={getTitle()}
      >
        <Settings size={14} />
      </button>
      {overlayContent && createPortal(overlayContent, document.body)}
    </div>
  );
}
