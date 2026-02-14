import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useAccountStore } from '../stores/useAccountStore';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { useGitHubCopilotAccountStore } from '../stores/useGitHubCopilotAccountStore';
import { useWindsurfAccountStore } from '../stores/useWindsurfAccountStore';
import { usePlatformLayoutStore } from '../stores/usePlatformLayoutStore';
import { Page } from '../types/navigation';
import { Users, CheckCircle2, Sparkles, RotateCw, Play, Github } from 'lucide-react';
import { getSubscriptionTier, getDisplayModels, getModelShortName, formatResetTimeDisplay } from '../utils/account';
import { getCodexPlanDisplayName, getCodexQuotaClass, formatCodexResetTime } from '../types/codex';
import { Account } from '../types/account';
import { CodexAccount } from '../types/codex';
import {
  GitHubCopilotAccount,
  getGitHubCopilotPlanDisplayName,
  getGitHubCopilotQuotaClass,
  formatGitHubCopilotResetTime,
} from '../types/githubCopilot';
import {
  WindsurfAccount,
  getWindsurfCreditsSummary,
  getWindsurfPlanDisplayName,
  getWindsurfQuotaClass,
  formatWindsurfResetTime,
} from '../types/windsurf';
import './DashboardPage.css';
import { RobotIcon } from '../components/icons/RobotIcon';
import { CodexIcon } from '../components/icons/CodexIcon';
import { WindsurfIcon } from '../components/icons/WindsurfIcon';
import { PlatformId, PLATFORM_PAGE_MAP } from '../types/platform';
import { getPlatformLabel, renderPlatformIcon } from '../utils/platformMeta';

interface DashboardPageProps {
  onNavigate: (page: Page) => void;
  onOpenPlatformLayout: () => void;
  onEasterEggTriggerClick: () => void;
}

const GHCP_CURRENT_ACCOUNT_ID_KEY = 'agtools.github_copilot.current_account_id';
const WINDSURF_CURRENT_ACCOUNT_ID_KEY = 'agtools.windsurf.current_account_id';

function toFiniteNumber(value: number | null | undefined): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function formatDecimal(value: number | null | undefined): string {
  const safe = toFiniteNumber(value);
  return (safe ?? 0).toFixed(2);
}

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  if (value <= 0) return 0;
  if (value >= 100) return 100;
  return Math.round(value);
}

function buildCreditMetrics(
  used: number | null | undefined,
  total: number | null | undefined,
  left: number | null | undefined,
) {
  const safeUsed = toFiniteNumber(used);
  const safeTotal = toFiniteNumber(total);
  const safeLeft = toFiniteNumber(left);

  let usedPercent = 0;
  if (safeTotal != null && safeTotal > 0) {
    if (safeUsed != null) {
      usedPercent = clampPercent((safeUsed / safeTotal) * 100);
    } else if (safeLeft != null) {
      usedPercent = clampPercent(((safeTotal - safeLeft) / safeTotal) * 100);
    }
  }

  return {
    usedPercent,
    used: safeUsed ?? 0,
    total: safeTotal ?? 0,
    left: safeLeft ?? 0,
  };
}

export function DashboardPage({ onNavigate, onOpenPlatformLayout, onEasterEggTriggerClick }: DashboardPageProps) {
  const { t } = useTranslation();
  const { orderedPlatformIds, hiddenPlatformIds } = usePlatformLayoutStore();
  const visiblePlatformOrder = useMemo(
    () => orderedPlatformIds.filter((platformId) => !hiddenPlatformIds.includes(platformId)),
    [orderedPlatformIds, hiddenPlatformIds],
  );

  
  // Antigravity Data
  const { 
    accounts: agAccounts, 
    currentAccount: agCurrent,
    switchAccount: switchAgAccount,
    fetchAccounts: fetchAgAccounts,
    fetchCurrentAccount: fetchAgCurrent
  } = useAccountStore();

  // Codex Data
  const { 
    accounts: codexAccounts, 
    currentAccount: codexCurrent,
    switchAccount: switchCodexAccount,
    fetchAccounts: fetchCodexAccounts,
    fetchCurrentAccount: fetchCodexCurrent
  } = useCodexAccountStore();

  // GitHub Copilot Data
  const {
    accounts: githubCopilotAccounts,
    fetchAccounts: fetchGitHubCopilotAccounts,
    switchAccount: switchGitHubCopilotAccount,
  } = useGitHubCopilotAccountStore();

  // Windsurf Data
  const {
    accounts: windsurfAccounts,
    fetchAccounts: fetchWindsurfAccounts,
    switchAccount: switchWindsurfAccount,
  } = useWindsurfAccountStore();

  const agCurrentId = agCurrent?.id;
  const codexCurrentId = codexCurrent?.id;

  const agCurrentAccount = useMemo(() => {
    if (!agCurrentId) return null;
    return agAccounts.find((account) => account.id === agCurrentId) ?? agCurrent ?? null;
  }, [agAccounts, agCurrent, agCurrentId]);

  const codexCurrentAccount = useMemo(() => {
    if (!codexCurrentId) return null;
    return codexAccounts.find((account) => account.id === codexCurrentId) ?? codexCurrent ?? null;
  }, [codexAccounts, codexCurrent, codexCurrentId]);

  React.useEffect(() => {
    fetchAgAccounts();
    fetchAgCurrent();
    fetchCodexAccounts();
    fetchCodexCurrent();
    fetchGitHubCopilotAccounts();
    fetchWindsurfAccounts();
  }, []);

  // Statistics
  const stats = useMemo(() => {
    return {
      total: agAccounts.length + codexAccounts.length + githubCopilotAccounts.length + windsurfAccounts.length,
      antigravity: agAccounts.length,
      codex: codexAccounts.length,
      githubCopilot: githubCopilotAccounts.length,
      windsurf: windsurfAccounts.length,
    };
  }, [agAccounts, codexAccounts, githubCopilotAccounts, windsurfAccounts]);

  // Refresh States
  const [refreshing, setRefreshing] = React.useState<Set<string>>(new Set());
  const [switching, setSwitching] = React.useState<Set<string>>(new Set());
  const [githubCopilotCurrentId, setGitHubCopilotCurrentId] = React.useState<string | null>(() => {
    try {
      return localStorage.getItem(GHCP_CURRENT_ACCOUNT_ID_KEY);
    } catch {
      return null;
    }
  });
  const [windsurfCurrentId, setWindsurfCurrentId] = React.useState<string | null>(() => {
    try {
      return localStorage.getItem(WINDSURF_CURRENT_ACCOUNT_ID_KEY);
    } catch {
      return null;
    }
  });
  const [cardRefreshing, setCardRefreshing] = React.useState<{
    ag: boolean;
    codex: boolean;
    githubCopilot: boolean;
    windsurf: boolean;
  }>({
    ag: false,
    codex: false,
    githubCopilot: false,
    windsurf: false,
  });

  // Refresh Handlers
  const handleRefreshAg = async (accountId: string) => {
    if (refreshing.has(accountId)) return;
    setRefreshing(prev => new Set(prev).add(accountId));
    try {
      await useAccountStore.getState().refreshQuota(accountId);
    } catch (error) {
      console.error('Refresh failed:', error);
    } finally {
      setRefreshing(prev => {
        const next = new Set(prev);
        next.delete(accountId);
        return next;
      });
    }
  };

  const handleRefreshCodex = async (accountId: string) => {
    if (refreshing.has(accountId)) return;
    setRefreshing(prev => new Set(prev).add(accountId));
    try {
      await useCodexAccountStore.getState().refreshQuota(accountId);
    } catch (error) {
      console.error('Refresh failed:', error);
    } finally {
      setRefreshing(prev => {
        const next = new Set(prev);
        next.delete(accountId);
        return next;
      });
    }
  };

  const handleRefreshGitHubCopilot = async (accountId: string) => {
    if (refreshing.has(accountId)) return;
    setRefreshing(prev => new Set(prev).add(accountId));
    try {
      await useGitHubCopilotAccountStore.getState().refreshToken(accountId);
    } catch (error) {
      console.error('Refresh failed:', error);
    } finally {
      setRefreshing(prev => {
        const next = new Set(prev);
        next.delete(accountId);
        return next;
      });
    }
  };

  const handleRefreshWindsurf = async (accountId: string) => {
    if (refreshing.has(accountId)) return;
    setRefreshing((prev) => new Set(prev).add(accountId));
    try {
      await useWindsurfAccountStore.getState().refreshToken(accountId);
    } catch (error) {
      console.error('Refresh failed:', error);
    } finally {
      setRefreshing((prev) => {
        const next = new Set(prev);
        next.delete(accountId);
        return next;
      });
    }
  };

  const handleRefreshAgCard = async () => {
    if (cardRefreshing.ag) return;
    setCardRefreshing(prev => ({ ...prev, ag: true }));
    const idsToRefresh = [agCurrentId, agRecommended?.id].filter(Boolean) as string[];
    try {
      for (const id of idsToRefresh) {
        await useAccountStore.getState().refreshQuota(id);
      }
    } catch (error) {
      console.error('Card refresh failed:', error);
    } finally {
      setCardRefreshing(prev => ({ ...prev, ag: false }));
    }
  };

  const handleRefreshCodexCard = async () => {
    if (cardRefreshing.codex) return;
    setCardRefreshing(prev => ({ ...prev, codex: true }));
    const idsToRefresh = [codexCurrentId, codexRecommended?.id].filter(Boolean) as string[];
    try {
      for (const id of idsToRefresh) {
        await useCodexAccountStore.getState().refreshQuota(id);
      }
    } catch (error) {
      console.error('Card refresh failed:', error);
    } finally {
      setCardRefreshing(prev => ({ ...prev, codex: false }));
    }
  };

  const handleRefreshGitHubCopilotCard = async () => {
    if (cardRefreshing.githubCopilot) return;
    setCardRefreshing(prev => ({ ...prev, githubCopilot: true }));
    const idsToRefresh = [githubCopilotCurrent?.id, githubCopilotRecommended?.id].filter(Boolean) as string[];
    try {
      for (const id of idsToRefresh) {
        await useGitHubCopilotAccountStore.getState().refreshToken(id);
      }
    } catch (error) {
      console.error('Card refresh failed:', error);
    } finally {
      setCardRefreshing(prev => ({ ...prev, githubCopilot: false }));
    }
  };

  const handleRefreshWindsurfCard = async () => {
    if (cardRefreshing.windsurf) return;
    setCardRefreshing((prev) => ({ ...prev, windsurf: true }));
    const idsToRefresh = [windsurfCurrent?.id, windsurfRecommended?.id].filter(Boolean) as string[];
    try {
      for (const id of idsToRefresh) {
        await useWindsurfAccountStore.getState().refreshToken(id);
      }
    } catch (error) {
      console.error('Card refresh failed:', error);
    } finally {
      setCardRefreshing((prev) => ({ ...prev, windsurf: false }));
    }
  };

  const handleSwitchGitHubCopilot = async (accountId: string) => {
    if (switching.has(accountId)) return;
    setSwitching((prev) => new Set(prev).add(accountId));
    try {
      await switchGitHubCopilotAccount(accountId);
      setGitHubCopilotCurrentId(accountId);
      localStorage.setItem(GHCP_CURRENT_ACCOUNT_ID_KEY, accountId);
    } catch (error) {
      console.error('Switch failed:', error);
    } finally {
      setSwitching((prev) => {
        const next = new Set(prev);
        next.delete(accountId);
        return next;
      });
    }
  };

  const handleSwitchWindsurf = async (accountId: string) => {
    if (switching.has(accountId)) return;
    setSwitching((prev) => new Set(prev).add(accountId));
    try {
      await switchWindsurfAccount(accountId);
      setWindsurfCurrentId(accountId);
      localStorage.setItem(WINDSURF_CURRENT_ACCOUNT_ID_KEY, accountId);
    } catch (error) {
      console.error('Switch failed:', error);
    } finally {
      setSwitching((prev) => {
        const next = new Set(prev);
        next.delete(accountId);
        return next;
      });
    }
  };

  // Antigravity Recommendation Logic
  const agRecommended = useMemo(() => {
    if (agAccounts.length <= 1) return null;
    
    // Simple logic: find account with highest overall quota that isn't current
    const others = agAccounts.filter((a) => {
      if (a.id === agCurrentId) return false;
      if (a.disabled) return false;
      if (a.quota?.is_forbidden) return false;
      if (!a.quota?.models || a.quota.models.length === 0) return false;
      return true;
    });
    if (others.length === 0) return null;

    return others.reduce((prev, curr) => {
      // Calculate a score based on quotas
      const getScore = (acc: Account) => {
        if (!acc.quota?.models) return -1;
        // Average percentage of all models
        const total = acc.quota.models.reduce((sum, m) => sum + m.percentage, 0);
        return total / acc.quota.models.length;
      };
      
      return getScore(curr) > getScore(prev) ? curr : prev;
    });
  }, [agAccounts, agCurrentId]);

  // Codex Recommendation Logic
  const codexRecommended = useMemo(() => {
    if (codexAccounts.length <= 1) return null;

    const others = codexAccounts.filter((a) => {
      if (a.id === codexCurrentId) return false;
      if (!a.quota) return false;
      return true;
    });
    if (others.length === 0) return null;

    return others.reduce((prev, curr) => {
      const getScore = (acc: CodexAccount) => {
        if (!acc.quota) return -1;
        return (acc.quota.hourly_percentage + acc.quota.weekly_percentage) / 2;
      };
      return getScore(curr) > getScore(prev) ? curr : prev;
    });
  }, [codexAccounts, codexCurrentId]);

  const githubCopilotCurrent = useMemo(() => {
    if (githubCopilotAccounts.length === 0) return null;
    if (githubCopilotCurrentId) {
      const current = githubCopilotAccounts.find((account) => account.id === githubCopilotCurrentId);
      if (current) return current;
    }
    return githubCopilotAccounts.reduce((prev, curr) => {
      const prevScore = prev.last_used || prev.created_at || 0;
      const currScore = curr.last_used || curr.created_at || 0;
      return currScore > prevScore ? curr : prev;
    });
  }, [githubCopilotAccounts, githubCopilotCurrentId]);

  const windsurfCurrent = useMemo(() => {
    if (windsurfAccounts.length === 0) return null;
    if (windsurfCurrentId) {
      const current = windsurfAccounts.find((account) => account.id === windsurfCurrentId);
      if (current) return current;
    }
    return windsurfAccounts.reduce((prev, curr) => {
      const prevScore = prev.last_used || prev.created_at || 0;
      const currScore = curr.last_used || curr.created_at || 0;
      return currScore > prevScore ? curr : prev;
    });
  }, [windsurfAccounts, windsurfCurrentId]);

  React.useEffect(() => {
    if (!githubCopilotCurrentId) return;
    const exists = githubCopilotAccounts.some((account) => account.id === githubCopilotCurrentId);
    if (exists) return;
    setGitHubCopilotCurrentId(null);
    localStorage.removeItem(GHCP_CURRENT_ACCOUNT_ID_KEY);
  }, [githubCopilotAccounts, githubCopilotCurrentId]);

  React.useEffect(() => {
    if (!windsurfCurrentId) return;
    const exists = windsurfAccounts.some((account) => account.id === windsurfCurrentId);
    if (exists) return;
    setWindsurfCurrentId(null);
    localStorage.removeItem(WINDSURF_CURRENT_ACCOUNT_ID_KEY);
  }, [windsurfAccounts, windsurfCurrentId]);

  const githubCopilotRecommended = useMemo(() => {
    if (githubCopilotAccounts.length <= 1) return null;
    const currentId = githubCopilotCurrent?.id;
    const others = githubCopilotAccounts.filter((a) => a.id !== currentId);
    if (others.length === 0) return null;

    const getScore = (acc: GitHubCopilotAccount) => {
      const scores = [acc.quota?.hourly_percentage, acc.quota?.weekly_percentage].filter(
        (value): value is number => typeof value === 'number',
      );
      if (scores.length === 0) return 101;
      return scores.reduce((sum, value) => sum + value, 0) / scores.length;
    };

    return others.reduce((prev, curr) => (getScore(curr) < getScore(prev) ? curr : prev));
  }, [githubCopilotAccounts, githubCopilotCurrent?.id]);

  const windsurfRecommended = useMemo(() => {
    if (windsurfAccounts.length <= 1) return null;
    const currentId = windsurfCurrent?.id;
    const others = windsurfAccounts.filter((account) => account.id !== currentId);
    if (others.length === 0) return null;

    const getScore = (account: WindsurfAccount) => {
      const credits = getWindsurfCreditsSummary(account);
      const promptLeft = toFiniteNumber(credits.promptCreditsLeft);
      const addOnLeft = toFiniteNumber(credits.addOnCredits);

      if (promptLeft != null) {
        return promptLeft * 1000 + (addOnLeft ?? 0);
      }

      const quotaValues = [account.quota?.hourly_percentage, account.quota?.weekly_percentage].filter(
        (value): value is number => typeof value === 'number',
      );
      if (quotaValues.length > 0) {
        const avgUsed = quotaValues.reduce((sum, value) => sum + value, 0) / quotaValues.length;
        return 100 - avgUsed;
      }

      return (account.last_used || account.created_at || 0) / 1e9;
    };

    return others.reduce((prev, curr) => (getScore(curr) > getScore(prev) ? curr : prev));
  }, [windsurfAccounts, windsurfCurrent?.id]);

  // Render Helpers
  const renderAgAccountContent = (account: Account | null) => {
    if (!account) return <div className="empty-slot">{t('dashboard.noAccount', '无账号')}</div>;

    const tier = getSubscriptionTier(account.quota);
    const tierLabel = t(`accounts.tier.${tier.toLowerCase()}`, tier);
    const displayModels = getDisplayModels(account.quota).slice(0, 4); // Show top 4 models

    return (
      <div className="account-mini-card">
        <div className="account-mini-header">
           <div className="account-info-row">
             <span className="account-email" title={account.email}>{account.email}</span>
             <span className={`tier-tag ${tier.toLowerCase()}`}>{tierLabel}</span>
           </div>
        </div>
        
        <div className="account-mini-quotas">
          {displayModels.map(model => (
            <div key={model.name} className="mini-quota-row-stacked">
              <div className="mini-quota-header">
                <span className="model-name">{getModelShortName(model.name)}</span>
                <span className={`model-pct ${getQuotaClass(model.percentage)}`}>{model.percentage}%</span>
              </div>
              <div className="mini-progress-track">
                <div 
                  className={`mini-progress-bar ${getQuotaClass(model.percentage)}`}
                  style={{ width: `${model.percentage}%` }}
                />
              </div>
              {model.reset_time && (
                <div className="mini-reset-time">
                  {formatResetTimeDisplay(model.reset_time, t)}
                </div>
              )}
            </div>
          ))}
          {displayModels.length === 0 && <span className="no-data-text">{t('dashboard.noData', '暂无数据')}</span>}
        </div>

        <div className="account-mini-actions icon-only-row">
           <button 
             className="mini-icon-btn" 
             onClick={() => handleRefreshAg(account.id)}
             title={t('common.refresh', '刷新')}
             disabled={refreshing.has(account.id)}
           >
             <RotateCw size={14} className={refreshing.has(account.id) ? 'loading-spinner' : ''} />
           </button>
           <button 
             className="mini-icon-btn"
             onClick={() => switchAgAccount(account.id)}
             title={t('dashboard.switch', '切换')}
           >
             <Play size={14} />
           </button>
        </div>
      </div>
    );
  };

  const renderCodexAccountContent = (account: CodexAccount | null) => {
    if (!account) return <div className="empty-slot">{t('dashboard.noAccount', '无账号')}</div>;

    const planName = getCodexPlanDisplayName(account.plan_type);
    const planLabel = t(`codex.plan.${planName.toLowerCase()}`, planName);
    
    return (
      <div className="account-mini-card">
        <div className="account-mini-header">
           <div className="account-info-row">
             <span className="account-email" title={account.email}>{account.email}</span>
             <span className={`tier-tag ${planName.toLowerCase()}`}>{planLabel}</span>
           </div>
        </div>
        
        <div className="account-mini-quotas">
          <div className="mini-quota-row-stacked">
            <div className="mini-quota-header">
              <span className="model-name">{t('codex.quota.hourly', '5H')}</span>
              <span className={`model-pct ${getCodexQuotaClass(account.quota?.hourly_percentage ?? 100)}`}>
                {account.quota?.hourly_percentage ?? 100}%
              </span>
            </div>
            <div className="mini-progress-track">
              <div 
                className={`mini-progress-bar ${getCodexQuotaClass(account.quota?.hourly_percentage ?? 100)}`}
                style={{ width: `${account.quota?.hourly_percentage ?? 100}%` }}
              />
            </div>
            {account.quota?.hourly_reset_time && (
              <div className="mini-reset-time">
                {formatCodexResetTime(account.quota.hourly_reset_time, t)}
              </div>
            )}
          </div>

          <div className="mini-quota-row-stacked">
            <div className="mini-quota-header">
              <span className="model-name">{t('codex.quota.weekly', 'Week')}</span>
              <span className={`model-pct ${getCodexQuotaClass(account.quota?.weekly_percentage ?? 100)}`}>
                {account.quota?.weekly_percentage ?? 100}%
              </span>
            </div>
            <div className="mini-progress-track">
              <div 
                className={`mini-progress-bar ${getCodexQuotaClass(account.quota?.weekly_percentage ?? 100)}`}
                style={{ width: `${account.quota?.weekly_percentage ?? 100}%` }}
              />
            </div>
            {account.quota?.weekly_reset_time && (
              <div className="mini-reset-time">
                {formatCodexResetTime(account.quota.weekly_reset_time, t)}
              </div>
            )}
          </div>
        </div>

        <div className="account-mini-actions icon-only-row">
           <button 
             className="mini-icon-btn" 
             onClick={() => handleRefreshCodex(account.id)}
             title={t('common.refresh', '刷新')}
             disabled={refreshing.has(account.id)}
           >
             <RotateCw size={14} className={refreshing.has(account.id) ? 'loading-spinner' : ''} />
           </button>
           <button 
             className="mini-icon-btn"
             onClick={() => switchCodexAccount(account.id)}
             title={t('dashboard.switch', '切换')}
           >
             <Play size={14} />
           </button>
        </div>
      </div>
    );
  };

  const renderGitHubCopilotAccountContent = (account: GitHubCopilotAccount | null) => {
    if (!account) return <div className="empty-slot">{t('dashboard.noAccount', '无账号')}</div>;

    const planName = getGitHubCopilotPlanDisplayName(account.plan_type);
    const planLabel = t(`githubCopilot.plan.${planName.toLowerCase()}`, planName);
    const hourly = account.quota?.hourly_percentage ?? null;
    const weekly = account.quota?.weekly_percentage ?? null;
    const hasQuota = hourly != null || weekly != null;
    const isRefreshing = refreshing.has(account.id);
    const isSwitching = switching.has(account.id);

    return (
      <div className="account-mini-card">
        <div className="account-mini-header">
          <div className="account-info-row">
            <span className="account-email" title={account.email ?? account.github_email ?? account.github_login}>
              {account.email ?? account.github_email ?? account.github_login}
            </span>
            <span className={`tier-tag ${planName.toLowerCase()}`}>{planLabel}</span>
          </div>
        </div>

        <div className="account-mini-quotas">
          {!hasQuota && <span className="no-data-text">{t('dashboard.noData', '暂无数据')}</span>}
          {hasQuota && (
            <>
              <div className="mini-quota-row-stacked">
                <div className="mini-quota-header">
                  <span className="model-name">{t('githubCopilot.quota.hourly', 'Inline Suggestions')}</span>
                  <span className={`model-pct ${getGitHubCopilotQuotaClass(hourly ?? 0)}`}>
                    {hourly ?? 0}%
                  </span>
                </div>
                <div className="mini-progress-track">
                  <div
                    className={`mini-progress-bar ${getGitHubCopilotQuotaClass(hourly ?? 0)}`}
                    style={{ width: `${hourly ?? 0}%` }}
                  />
                </div>
                {account.quota?.hourly_reset_time && (
                  <div className="mini-reset-time">
                    {formatGitHubCopilotResetTime(account.quota.hourly_reset_time, t)}
                  </div>
                )}
              </div>

              <div className="mini-quota-row-stacked">
                <div className="mini-quota-header">
                  <span className="model-name">{t('githubCopilot.quota.weekly', 'Chat messages')}</span>
                  <span className={`model-pct ${getGitHubCopilotQuotaClass(weekly ?? 0)}`}>
                    {weekly ?? 0}%
                  </span>
                </div>
                <div className="mini-progress-track">
                  <div
                    className={`mini-progress-bar ${getGitHubCopilotQuotaClass(weekly ?? 0)}`}
                    style={{ width: `${weekly ?? 0}%` }}
                  />
                </div>
                {account.quota?.weekly_reset_time && (
                  <div className="mini-reset-time">
                    {formatGitHubCopilotResetTime(account.quota.weekly_reset_time, t)}
                  </div>
                )}
              </div>
            </>
          )}
        </div>

        <div className="account-mini-actions icon-only-row">
          <button
            className="mini-icon-btn"
            onClick={() => handleRefreshGitHubCopilot(account.id)}
            title={t('common.refresh', '刷新')}
            disabled={isRefreshing || isSwitching}
          >
            <RotateCw size={14} className={isRefreshing ? 'loading-spinner' : ''} />
          </button>
          <button
            className="mini-icon-btn"
            onClick={() => handleSwitchGitHubCopilot(account.id)}
            title={t('dashboard.switch', '切换')}
            disabled={isSwitching}
          >
            {isSwitching ? <RotateCw size={14} className="loading-spinner" /> : <Play size={14} />}
          </button>
        </div>
      </div>
    );
  };

  const renderWindsurfAccountContent = (account: WindsurfAccount | null) => {
    if (!account) return <div className="empty-slot">{t('dashboard.noAccount', '无账号')}</div>;

    const planName = getWindsurfPlanDisplayName(account.plan_type ?? account.copilot_plan);
    const planLabel = t(`windsurf.plan.${planName.toLowerCase()}`, planName);
    const credits = getWindsurfCreditsSummary(account);
    const promptMetrics = buildCreditMetrics(
      credits.promptCreditsUsed,
      credits.promptCreditsTotal,
      credits.promptCreditsLeft,
    );
    const addOnMetrics = buildCreditMetrics(credits.addOnCreditsUsed, credits.addOnCreditsTotal, credits.addOnCredits);
    const isRefreshing = refreshing.has(account.id);
    const isSwitching = switching.has(account.id);
    const cycleText = credits.planEndsAt
      ? formatWindsurfResetTime(credits.planEndsAt, t)
      : t('windsurf.credits.planEndsUnknown', '配额周期时间未知');

    return (
      <div className="account-mini-card">
        <div className="account-mini-header">
          <div className="account-info-row">
            <span className="account-email" title={account.email ?? account.github_email ?? account.github_login}>
              {account.email ?? account.github_email ?? account.github_login}
            </span>
            <span className={`tier-tag ${planName.toLowerCase()}`}>{planLabel}</span>
          </div>
        </div>

        <div className="account-mini-quotas">
          <div className="mini-quota-row-stacked">
            <div className="mini-quota-header">
              <span className="model-name">{t('windsurf.columns.promptCredits', 'User Prompt credits')}</span>
              <span className={`model-pct ${getWindsurfQuotaClass(promptMetrics.usedPercent)}`}>
                {promptMetrics.usedPercent}%
              </span>
            </div>
            <div className="mini-progress-track">
              <div
                className={`mini-progress-bar ${getWindsurfQuotaClass(promptMetrics.usedPercent)}`}
                style={{ width: `${promptMetrics.usedPercent}%` }}
              />
            </div>
            <div className="mini-reset-time">
              {t('windsurf.credits.usedLine', {
                used: formatDecimal(promptMetrics.used),
                total: formatDecimal(promptMetrics.total),
                defaultValue: '{{used}} / {{total}} used',
              })}
            </div>
            <div className="mini-reset-time">
              {t('windsurf.credits.leftInline', {
                left: formatDecimal(promptMetrics.left),
                defaultValue: '{{left}} left',
              })}
            </div>
          </div>

          <div className="mini-quota-row-stacked">
            <div className="mini-quota-header">
              <span className="model-name">{t('windsurf.columns.addOnPromptCredits', 'Add-on prompt credits')}</span>
              <span className={`model-pct ${getWindsurfQuotaClass(addOnMetrics.usedPercent)}`}>
                {addOnMetrics.usedPercent}%
              </span>
            </div>
            <div className="mini-progress-track">
              <div
                className={`mini-progress-bar ${getWindsurfQuotaClass(addOnMetrics.usedPercent)}`}
                style={{ width: `${addOnMetrics.usedPercent}%` }}
              />
            </div>
            <div className="mini-reset-time">
              {t('windsurf.credits.usedLine', {
                used: formatDecimal(addOnMetrics.used),
                total: formatDecimal(addOnMetrics.total),
                defaultValue: '{{used}} / {{total}} used',
              })}
            </div>
            <div className="mini-reset-time">
              {t('windsurf.credits.leftInline', {
                left: formatDecimal(addOnMetrics.left),
                defaultValue: '{{left}} left',
              })}
            </div>
          </div>

          <div className="mini-cycle-time">{cycleText}</div>
        </div>

        <div className="account-mini-actions icon-only-row">
          <button
            className="mini-icon-btn"
            onClick={() => handleRefreshWindsurf(account.id)}
            title={t('common.refresh', '刷新')}
            disabled={isRefreshing || isSwitching}
          >
            <RotateCw size={14} className={isRefreshing ? 'loading-spinner' : ''} />
          </button>
          <button
            className="mini-icon-btn"
            onClick={() => handleSwitchWindsurf(account.id)}
            title={t('dashboard.switch', '切换')}
            disabled={isSwitching}
          >
            {isSwitching ? <RotateCw size={14} className="loading-spinner" /> : <Play size={14} />}
          </button>
        </div>
      </div>
    );
  };

  // Helper for Quota Class (duplicated from Account utils roughly)
  function getQuotaClass(percentage: number): string {
    if (percentage > 80) return 'high';
    if (percentage > 20) return 'medium';
    return 'low';
  }

  const platformCounts: Record<PlatformId, number> = {
    antigravity: stats.antigravity,
    codex: stats.codex,
    'github-copilot': stats.githubCopilot,
    windsurf: stats.windsurf,
  };

  const visibleCardPlatformIds = visiblePlatformOrder;
  const cardRows = useMemo(() => {
    const rows: PlatformId[][] = [];
    for (let i = 0; i < visibleCardPlatformIds.length; i += 2) {
      rows.push(visibleCardPlatformIds.slice(i, i + 2));
    }
    return rows;
  }, [visibleCardPlatformIds]);

  const renderPlatformCard = (platformId: PlatformId) => {
    if (platformId === 'antigravity') {
      return (
        <div className="main-card antigravity-card" key={platformId}>
          <div className="main-card-header">
            <div className="header-title">
              <RobotIcon className="" style={{ width: 18, height: 18 }} />
              <h3>{getPlatformLabel(platformId, t)}</h3>
            </div>
            <button
              className="header-action-btn"
              onClick={handleRefreshAgCard}
              disabled={cardRefreshing.ag}
              title={t('common.refresh', '刷新')}
            >
              <RotateCw size={14} className={cardRefreshing.ag ? 'loading-spinner' : ''} />
              <span>{t('common.refresh', '刷新')}</span>
            </button>
          </div>

          <div className="split-content">
            <div className="split-half current-half">
              <span className="half-label"><CheckCircle2 size={12} /> {t('dashboard.current', '当前账户')}</span>
              {renderAgAccountContent(agCurrentAccount)}
            </div>

            <div className="split-divider"></div>

            <div className="split-half recommend-half">
              <span className="half-label"><Sparkles size={12} /> {t('dashboard.recommended', '推荐账号')}</span>
              {agRecommended ? (
                renderAgAccountContent(agRecommended)
              ) : (
                <div className="empty-slot-text">{t('dashboard.noRecommendation', '暂无更好推荐')}</div>
              )}
            </div>
          </div>

          <button className="card-footer-action" onClick={() => onNavigate('overview')}>
            {t('dashboard.viewAllAccounts', '查看所有账号')}
          </button>
        </div>
      );
    }

    if (platformId === 'codex') {
      return (
        <div className="main-card codex-card" key={platformId}>
          <div className="main-card-header">
            <div className="header-title">
              <CodexIcon size={18} />
              <h3>{getPlatformLabel(platformId, t)}</h3>
            </div>
            <button
              className="header-action-btn"
              onClick={handleRefreshCodexCard}
              disabled={cardRefreshing.codex}
              title={t('common.refresh', '刷新')}
            >
              <RotateCw size={14} className={cardRefreshing.codex ? 'loading-spinner' : ''} />
              <span>{t('common.refresh', '刷新')}</span>
            </button>
          </div>

          <div className="split-content">
            <div className="split-half current-half">
              <span className="half-label"><CheckCircle2 size={12} /> {t('dashboard.current', '当前账户')}</span>
              {renderCodexAccountContent(codexCurrentAccount)}
            </div>

            <div className="split-divider"></div>

            <div className="split-half recommend-half">
              <span className="half-label"><Sparkles size={12} /> {t('dashboard.recommended', '推荐账号')}</span>
              {codexRecommended ? (
                renderCodexAccountContent(codexRecommended)
              ) : (
                <div className="empty-slot-text">{t('dashboard.noRecommendation', '暂无更好推荐')}</div>
              )}
            </div>
          </div>

          <button className="card-footer-action" onClick={() => onNavigate('codex')}>
            {t('dashboard.viewAllAccounts', '查看所有账号')}
          </button>
        </div>
      );
    }

    if (platformId === 'github-copilot') {
      return (
        <div className="main-card github-copilot-card" key={platformId}>
          <div className="main-card-header">
            <div className="header-title">
              <Github size={18} />
              <h3>{getPlatformLabel(platformId, t)}</h3>
            </div>
            <button
              className="header-action-btn"
              onClick={handleRefreshGitHubCopilotCard}
              disabled={cardRefreshing.githubCopilot}
              title={t('common.refresh', '刷新')}
            >
              <RotateCw size={14} className={cardRefreshing.githubCopilot ? 'loading-spinner' : ''} />
              <span>{t('common.refresh', '刷新')}</span>
            </button>
          </div>

          <div className="split-content">
            <div className="split-half current-half">
              <span className="half-label"><CheckCircle2 size={12} /> {t('dashboard.current', '当前账户')}</span>
              {renderGitHubCopilotAccountContent(githubCopilotCurrent)}
            </div>

            <div className="split-divider"></div>

            <div className="split-half recommend-half">
              <span className="half-label"><Sparkles size={12} /> {t('dashboard.recommended', '推荐账号')}</span>
              {githubCopilotRecommended ? (
                renderGitHubCopilotAccountContent(githubCopilotRecommended)
              ) : (
                <div className="empty-slot-text">{t('dashboard.noRecommendation', '暂无更好推荐')}</div>
              )}
            </div>
          </div>

          <button className="card-footer-action" onClick={() => onNavigate('github-copilot')}>
            {t('dashboard.viewAllAccounts', '查看所有账号')}
          </button>
        </div>
      );
    }

    return (
      <div className="main-card windsurf-card" key={platformId}>
        <div className="main-card-header">
          <div className="header-title">
            <WindsurfIcon className="" style={{ width: 18, height: 18 }} />
            <h3>Windsurf</h3>
          </div>
          <button
            className="header-action-btn"
            onClick={handleRefreshWindsurfCard}
            disabled={cardRefreshing.windsurf}
            title={t('common.refresh', '刷新')}
          >
            <RotateCw size={14} className={cardRefreshing.windsurf ? 'loading-spinner' : ''} />
            <span>{t('common.refresh', '刷新')}</span>
          </button>
        </div>

        <div className="split-content">
          <div className="split-half current-half">
            <span className="half-label"><CheckCircle2 size={12} /> {t('dashboard.current', '当前账户')}</span>
            {renderWindsurfAccountContent(windsurfCurrent)}
          </div>

          <div className="split-divider"></div>

          <div className="split-half recommend-half">
            <span className="half-label"><Sparkles size={12} /> {t('dashboard.recommended', '推荐账号')}</span>
            {windsurfRecommended ? (
              renderWindsurfAccountContent(windsurfRecommended)
            ) : (
              <div className="empty-slot-text">{t('dashboard.noRecommendation', '暂无更好推荐')}</div>
            )}
          </div>
        </div>

        <button className="card-footer-action" onClick={() => onNavigate('windsurf')}>
          {t('dashboard.viewAllAccounts', '查看所有账号')}
        </button>
      </div>
    );
  };

  return (
    <main className="main-content dashboard-page fade-in">
      <div className="page-tabs-row" style={{ minHeight: '60px' }}>
         <div className="page-tabs-label">{t('nav.dashboard', '仪表盘')}</div>
         <div className="dashboard-top-actions">
           <button className="header-action-btn" onClick={onOpenPlatformLayout}>
             <span>{t('platformLayout.title', '平台布局')}</span>
           </button>
           <span className="date-display">{new Date().toLocaleDateString()}</span>
         </div>
      </div>

      {/* Top Stats */}
      <div className="stats-row">
        <div className="stat-card">
          <div className="stat-icon-bg primary"><Users size={24} /></div>
          <div className="stat-info">
            <span className="stat-label">{t('dashboard.totalAccounts', '账号总数')}</span>
            <span className="stat-value">{stats.total}</span>
          </div>
        </div>

        {visiblePlatformOrder.map((platformId) => {
          const iconClass =
            platformId === 'antigravity'
              ? 'success'
              : platformId === 'codex'
              ? 'info'
              : platformId === 'github-copilot'
              ? 'github'
              : 'windsurf';
          return (
            <button
              className="stat-card stat-card-button"
              key={platformId}
              onClick={() => onNavigate(PLATFORM_PAGE_MAP[platformId])}
              title={t('dashboard.switchTo', '切换到此账号')}
            >
              <div
                className={`stat-icon-bg ${iconClass} stat-icon-trigger`}
                onClick={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  onEasterEggTriggerClick();
                }}
              >
                {renderPlatformIcon(platformId, 24)}
              </div>
              <div className="stat-info">
                <span className="stat-label">{getPlatformLabel(platformId, t)}</span>
                <span className="stat-value">{platformCounts[platformId]}</span>
              </div>
            </button>
          );
        })}
      </div>

      {/* Main Comparison Section */}
      <div className="cards-section">
        {cardRows.map((row, rowIndex) => (
          <div className="cards-split-row" key={`row-${rowIndex}`}>
            {row.map((platformId) => renderPlatformCard(platformId))}
            {row.length < 2 && <div className="main-card main-card-placeholder" />}
          </div>
        ))}
      </div>

    </main>
  );
}
