type BridgeInvokeResponse =
  | { ok: true; value: unknown }
  | { ok: false; error: unknown };

type CallbackEntry = {
  callback: (...args: unknown[]) => void;
  once: boolean;
};

const bridgeAnyWindow = window as unknown as {
  __TAURI_INTERNALS__?: {
    metadata: {
      currentWindow: { label: string };
      currentWebview: { label: string };
    };
    invoke: (cmd: string, args?: Record<string, unknown>, options?: unknown) => Promise<unknown>;
    transformCallback: (callback: (...args: unknown[]) => void, once?: boolean) => number;
    unregisterCallback: (id: number) => void;
  };
};

if (!bridgeAnyWindow.__TAURI_INTERNALS__) {
  const callbacks = new Map<number, CallbackEntry>();
  let nextCallbackId = 1;
  let nextEventId = 1;

  const callBridge = async (cmd: string, args: Record<string, unknown> = {}) => {
    const response = await fetch('/__cockpit_web__/invoke', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({ cmd, args }),
    });

    const payload = (await response.json()) as BridgeInvokeResponse;
    if (!response.ok || !payload.ok) {
      const rawError = payload && 'error' in payload ? payload.error : response.statusText;
      throw rawError instanceof Error ? rawError : new Error(String(rawError || 'Web invoke failed'));
    }
    return payload.value;
  };

  const invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
    switch (cmd) {
      case 'plugin:event|listen':
        return nextEventId++;
      case 'plugin:event|unlisten':
      case 'plugin:event|emit':
      case 'plugin:event|emit_to':
      case 'plugin:window|start_dragging':
      case 'plugin:window|set_theme':
      case 'plugin:webview|set_webview_zoom':
      case 'plugin:webview|set_zoom':
        return null;
      case 'plugin:window|get_all_windows':
        return [{ label: 'main' }];
      case 'plugin:webview|get_all_webviews':
        return [{ label: 'main', windowLabel: 'main' }];
      case 'plugin:dialog|open':
      case 'plugin:dialog|save':
        return null;
      case 'plugin:dialog|message':
        window.alert(String(args.message ?? ''));
        return null;
      case 'plugin:dialog|ask':
      case 'plugin:dialog|confirm':
        return window.confirm(String(args.message ?? ''));
      case 'plugin:opener|open_url':
      case 'plugin:opener|openUrl': {
        const target = String(args.url ?? args.path ?? '');
        if (target) {
          window.open(target, '_blank', 'noopener,noreferrer');
        }
        return null;
      }
      case 'plugin:opener|open_path':
      case 'plugin:opener|openPath':
        return null;
      case 'plugin:updater|check':
        return null;
      case 'plugin:updater|download':
      case 'plugin:updater|install':
      case 'plugin:updater|download_and_install':
        throw new Error('Updater actions are only available in the desktop app.');
      case 'plugin:process|restart':
      case 'plugin:process|relaunch':
        throw new Error('This action is only available in the desktop app.');
      default:
        return callBridge(cmd, args);
    }
  };

  bridgeAnyWindow.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: 'main' },
      currentWebview: { label: 'main' },
    },
    invoke,
    transformCallback(callback, once = false) {
      const id = nextCallbackId++;
      callbacks.set(id, { callback, once });
      return id;
    },
    unregisterCallback(id) {
      callbacks.delete(id);
    },
  };
}
