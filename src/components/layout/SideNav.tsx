import { Settings, Rocket, GaugeCircle, LayoutGrid, SlidersHorizontal } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useState, useRef, useCallback, useEffect, useMemo } from 'react';
import { Page } from '../../types/navigation';
import { PlatformId, PLATFORM_PAGE_MAP } from '../../types/platform';
import { usePlatformLayoutStore } from '../../stores/usePlatformLayoutStore';
import { getPlatformLabel, renderPlatformIcon } from '../../utils/platformMeta';

interface SideNavProps {
  page: Page;
  setPage: (page: Page) => void;
  onOpenPlatformLayout: () => void;
  easterEggClickCount: number;
  onEasterEggTriggerClick: () => void;
}

interface FlyingRocket {
  id: number;
  x: number;
}

const PAGE_PLATFORM_MAP: Partial<Record<Page, PlatformId>> = {
  overview: 'antigravity',
  codex: 'codex',
  'github-copilot': 'github-copilot',
  windsurf: 'windsurf',
};

export function SideNav({
  page,
  setPage,
  onOpenPlatformLayout,
  easterEggClickCount,
  onEasterEggTriggerClick,
}: SideNavProps) {
  const { t } = useTranslation();
  const [flyingRockets, setFlyingRockets] = useState<FlyingRocket[]>([]);
  const [showMore, setShowMore] = useState(false);
  const rocketIdRef = useRef(0);
  const logoRef = useRef<HTMLDivElement>(null);
  const morePopoverRef = useRef<HTMLDivElement>(null);
  const moreButtonRef = useRef<HTMLButtonElement>(null);
  const { orderedPlatformIds, hiddenPlatformIds, sidebarPlatformIds } = usePlatformLayoutStore();

  const currentPlatformId = PAGE_PLATFORM_MAP[page] ?? null;
  const hiddenSet = useMemo(() => new Set(hiddenPlatformIds), [hiddenPlatformIds]);
  const sidebarVisiblePlatformIds = useMemo(
    () => orderedPlatformIds.filter((id) => sidebarPlatformIds.includes(id) && !hiddenSet.has(id)),
    [orderedPlatformIds, sidebarPlatformIds, hiddenSet],
  );
  const isMoreActive = !!currentPlatformId && !sidebarVisiblePlatformIds.includes(currentPlatformId);

  const handleLogoClick = useCallback(() => {
    const newRocket: FlyingRocket = {
      id: rocketIdRef.current++,
      x: (Math.random() - 0.5) * 40, // 随机水平偏移
    };

    setFlyingRockets(prev => [...prev, newRocket]);

    // 动画完成后移除火箭 (1.5秒)
    setTimeout(() => {
      setFlyingRockets(prev => prev.filter(r => r.id !== newRocket.id));
    }, 1500);

    onEasterEggTriggerClick();
  }, [onEasterEggTriggerClick]);

  useEffect(() => {
    if (!showMore) return;
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (morePopoverRef.current?.contains(target)) return;
      if (moreButtonRef.current?.contains(target)) return;
      setShowMore(false);
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [showMore]);

  return (
    <nav className="side-nav">
      <div className="nav-brand" style={{ position: 'relative', zIndex: 10 }}>
         <div 
           ref={logoRef}
           className="brand-logo rocket-easter-egg" 
           onClick={handleLogoClick}
         >
           <Rocket size={20} />
           {/* 点击计数器保持在里面，跟随缩放 */}
           {easterEggClickCount > 0 && (
             <span className="rocket-click-count">{easterEggClickCount}</span>
           )}
         </div>

         {/* 把火箭层移到外面，放在后面以自然层叠在上方，使用 pointer-events-none 防止遮挡点击 */}
         <div style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', pointerEvents: 'none' }}>
           {flyingRockets.map(rocket => (
             <span 
               key={rocket.id} 
               className="flying-rocket"
               style={{ '--rocket-x': `${rocket.x}px` } as React.CSSProperties}
             >
               🚀
             </span>
           ))}
         </div>
      </div>
      
      <div className="nav-items">

        <button 
          className={`nav-item ${page === 'dashboard' ? 'active' : ''}`} 
          onClick={() => setPage('dashboard')}
          title={t('nav.dashboard')}
        >
          <GaugeCircle size={20} />
          <span className="tooltip">{t('nav.dashboard')}</span>
        </button>

        {sidebarVisiblePlatformIds.map((platformId) => {
          const active = currentPlatformId === platformId;
          return (
            <button
              key={platformId}
              className={`nav-item ${active ? 'active' : ''}`}
              onClick={() => setPage(PLATFORM_PAGE_MAP[platformId])}
              title={getPlatformLabel(platformId, t)}
            >
              {renderPlatformIcon(platformId, 20)}
              <span className="tooltip">{getPlatformLabel(platformId, t)}</span>
            </button>
          );
        })}

        <button
          ref={moreButtonRef}
          className={`nav-item ${showMore || isMoreActive ? 'active' : ''}`}
          onClick={() => setShowMore((prev) => !prev)}
          title={t('nav.morePlatforms', '更多平台')}
        >
          <LayoutGrid size={20} />
          <span className="tooltip">{t('nav.morePlatforms', '更多平台')}</span>
        </button>

        {showMore && (
          <div className="side-nav-more-popover" ref={morePopoverRef}>
            <div className="side-nav-more-title">{t('nav.morePlatforms', '更多平台')}</div>
            <div className="side-nav-more-list">
              {orderedPlatformIds.map((platformId) => {
                const active = currentPlatformId === platformId;
                const hidden = hiddenSet.has(platformId);
                return (
                  <button
                    key={platformId}
                    className={`side-nav-more-item ${active ? 'active' : ''}`}
                    onClick={() => {
                      setPage(PLATFORM_PAGE_MAP[platformId]);
                      setShowMore(false);
                    }}
                  >
                    <span className="side-nav-more-item-icon">{renderPlatformIcon(platformId, 16)}</span>
                    <span className="side-nav-more-item-label">{getPlatformLabel(platformId, t)}</span>
                    {hidden && <span className="side-nav-more-item-badge">{t('platformLayout.hiddenBadge', '已隐藏')}</span>}
                  </button>
                );
              })}
            </div>
            <button
              className="side-nav-more-manage"
              onClick={() => {
                setShowMore(false);
                onOpenPlatformLayout();
              }}
            >
              <SlidersHorizontal size={14} />
              <span>{t('platformLayout.openFromMore', '管理平台布局')}</span>
            </button>
          </div>
        )}
      </div>

      <div className="nav-footer">
        <button
          className={`nav-item ${page === 'settings' ? 'active' : ''}`}
          onClick={() => setPage('settings')}
          title={t('nav.settings')}
        >
          <Settings size={20} />
          <span className="tooltip">{t('nav.settings')}</span>
        </button>
      </div>

    </nav>
  );
}
