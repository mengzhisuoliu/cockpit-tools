import { X } from 'lucide-react';
import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useGlobalModalStore, type GlobalModalAction } from '../stores/useGlobalModalStore';
import './GlobalModal.css';

function resolveActionClass(variant: GlobalModalAction['variant']): string {
  if (variant === 'danger') return 'btn btn-danger';
  if (variant === 'secondary') return 'btn btn-secondary';
  return 'btn btn-primary';
}

export function GlobalModal() {
  const { t } = useTranslation();
  const visible = useGlobalModalStore((state) => state.visible);
  const modal = useGlobalModalStore((state) => state.modal);
  const closeModal = useGlobalModalStore((state) => state.closeModal);

  const handleOverlayClick = useCallback(() => {
    if (!modal || modal.closeOnOverlay === false) return;
    closeModal();
  }, [closeModal, modal]);

  const handleActionClick = useCallback(async (action: GlobalModalAction) => {
    if (action.disabled) return;
    try {
      if (action.onClick) {
        await Promise.resolve(action.onClick());
      }
    } finally {
      if (action.autoClose !== false) {
        closeModal();
      }
    }
  }, [closeModal]);

  if (!visible || !modal) return null;

  const actions = modal.actions && modal.actions.length > 0
    ? modal.actions
    : [
        {
          id: 'default-ok',
          label: t('globalModal.ok', '知道了'),
          variant: 'primary' as const,
        },
      ];

  const modalSizeClass = modal.width === 'lg'
    ? 'modal modal-lg'
    : modal.width === 'sm'
      ? 'modal global-modal-sm'
      : 'modal';

  return (
    <div className="modal-overlay global-modal-overlay" onClick={handleOverlayClick}>
      <div className={modalSizeClass} onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>{modal.title || t('globalModal.title', '提示')}</h2>
          {modal.showCloseButton !== false && (
            <button
              className="modal-close"
              onClick={closeModal}
              aria-label={t('common.close', '关闭')}
            >
              <X />
            </button>
          )}
        </div>

        <div className="modal-body global-modal-body">
          {modal.description && (
            <p className="global-modal-description">{modal.description}</p>
          )}
          {modal.content}
        </div>

        <div className="modal-footer global-modal-footer">
          {actions.map((action, index) => (
            <button
              key={action.id || `action-${index}`}
              className={resolveActionClass(action.variant)}
              onClick={() => { void handleActionClick(action); }}
              disabled={action.disabled}
            >
              {action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
