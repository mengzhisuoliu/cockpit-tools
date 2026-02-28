import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { useGitHubCopilotInstanceStore } from '../stores/useGitHubCopilotInstanceStore';
import { useGitHubCopilotAccountStore } from '../stores/useGitHubCopilotAccountStore';
import type { GitHubCopilotAccount } from '../types/githubCopilot';
import { getGitHubCopilotAccountDisplayEmail, getGitHubCopilotQuotaClass, getGitHubCopilotUsage } from '../types/githubCopilot';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';

/**
 * GitHub Copilot 多开实例内容组件（不包含 header）
 * 用于嵌入到 GitHubCopilotAccountsPage 中
 */
export function GitHubCopilotInstancesContent() {
  const { t } = useTranslation();
  const instanceStore = useGitHubCopilotInstanceStore();
  const { accounts, fetchAccounts } = useGitHubCopilotAccountStore();
  type AccountForSelect = GitHubCopilotAccount & { email: string };
  const accountsForSelect = useMemo(
    () =>
      accounts.map((acc) => ({
        ...acc,
        email: acc.email || getGitHubCopilotAccountDisplayEmail(acc),
      })) as AccountForSelect[],
    [accounts],
  );
  const isSupportedPlatform = usePlatformRuntimeSupport('desktop');

  const resolveQuotaClass = (percentage: number) => getGitHubCopilotQuotaClass(percentage);

  const renderGitHubCopilotQuotaPreview = (account: AccountForSelect) => {
    const usage = getGitHubCopilotUsage(account);
    const inlinePct = usage.inlineSuggestionsUsedPercent;
    const chatPct = usage.chatMessagesUsedPercent;
    const premiumPct = usage.premiumRequestsUsedPercent;
    if (inlinePct == null && chatPct == null && premiumPct == null) {
      return <span className="account-quota-empty">{t('instances.quota.empty', '暂无配额缓存')}</span>;
    }
    return (
      <div className="account-quota-preview">
        <span className="account-quota-item">
          <span className={`quota-dot ${resolveQuotaClass(inlinePct ?? 0)}`} />
          <span className={`quota-text ${resolveQuotaClass(inlinePct ?? 0)}`}>
            Inline Suggestions {inlinePct ?? '-'}%
          </span>
        </span>
        <span className="account-quota-item">
          <span className={`quota-dot ${resolveQuotaClass(chatPct ?? 0)}`} />
          <span className={`quota-text ${resolveQuotaClass(chatPct ?? 0)}`}>
            Chat messages {chatPct ?? '-'}%
          </span>
        </span>
        <span className="account-quota-item">
          <span className={`quota-dot ${resolveQuotaClass(premiumPct ?? 0)}`} />
          <span className={`quota-text ${resolveQuotaClass(premiumPct ?? 0)}`}>
            Premium requests {premiumPct ?? '-'}%
          </span>
        </span>
      </div>
    );
  };

  return (
    <PlatformInstancesContent<AccountForSelect>
      instanceStore={instanceStore}
      accounts={accountsForSelect}
      fetchAccounts={fetchAccounts}
      renderAccountQuotaPreview={renderGitHubCopilotQuotaPreview}
      getAccountSearchText={(account) => account.email}
      appType="vscode"
      isSupported={isSupportedPlatform}
      unsupportedTitleKey="common.shared.instances.unsupported.title"
      unsupportedTitleDefault="暂不支持当前系统"
      unsupportedDescKey="githubCopilot.instances.unsupported.descPlatform"
      unsupportedDescDefault="GitHub Copilot 多开实例仅支持 macOS、Windows 和 Linux。"
    />
  );
}
