import { invoke } from '@tauri-apps/api/core';

type DiagnosticLevel = 'info' | 'warn' | 'error';

const PREFIX = '[HangDiag]';
const DEFAULT_PAINT_WARN_MS = 500;
const DEFAULT_OPERATION_WARN_MS = 800;
const UI_HANG_INTERVAL_MS = 1000;
const UI_HANG_WARN_DRIFT_MS = 1500;
const UI_HANG_REPORT_THROTTLE_MS = 5000;

let uiHangDiagnosticsStarted = false;

function stringifyValue(value: unknown): string {
  if (typeof value === 'string') {
    return value.length > 120 ? `string(${value.length})` : value;
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  if (value == null) {
    return String(value);
  }
  if (Array.isArray(value)) {
    return `array(${value.length})`;
  }
  if (typeof value === 'object') {
    return 'object';
  }
  return typeof value;
}

function formatFields(fields?: Record<string, unknown>): string {
  if (!fields) return '';
  const parts = Object.entries(fields)
    .filter(([, value]) => value !== undefined)
    .map(([key, value]) => `${key}=${stringifyValue(value)}`);
  return parts.length > 0 ? ` ${parts.join(', ')}` : '';
}

export function logHangDiagnostic(
  level: DiagnosticLevel,
  message: string,
  fields?: Record<string, unknown>,
) {
  const text = `${PREFIX} ${message}${formatFields(fields)}`;
  if (level === 'error') {
    console.error(text);
  } else if (level === 'warn') {
    console.warn(text);
  } else {
    console.info(text);
  }
  void invoke('update_log', { level, message: text }).catch(() => {});
}

export function trackNextPaint(
  name: string,
  fields?: Record<string, unknown>,
  warnAfterMs = DEFAULT_PAINT_WARN_MS,
) {
  if (typeof window === 'undefined') return;
  const startedAt = performance.now();
  window.requestAnimationFrame(() => {
    window.setTimeout(() => {
      const elapsedMs = Math.round(performance.now() - startedAt);
      if (elapsedMs >= warnAfterMs) {
        logHangDiagnostic('warn', `${name} next paint slow`, {
          ...fields,
          elapsedMs,
          warnAfterMs,
        });
      } else {
        logHangDiagnostic('info', `${name} next paint`, {
          ...fields,
          elapsedMs,
        });
      }
    }, 0);
  });
}

export async function measureHangDiagnostic<T>(
  name: string,
  fields: Record<string, unknown> | undefined,
  operation: () => Promise<T>,
  warnAfterMs = DEFAULT_OPERATION_WARN_MS,
): Promise<T> {
  const startedAt = performance.now();
  logHangDiagnostic('info', `${name} start`, fields);
  try {
    const result = await operation();
    const elapsedMs = Math.round(performance.now() - startedAt);
    logHangDiagnostic(elapsedMs >= warnAfterMs ? 'warn' : 'info', `${name} done`, {
      ...fields,
      elapsedMs,
      warnAfterMs,
    });
    return result;
  } catch (error) {
    const elapsedMs = Math.round(performance.now() - startedAt);
    logHangDiagnostic('warn', `${name} failed`, {
      ...fields,
      elapsedMs,
      error: String(error).replace(/^Error:\s*/, ''),
    });
    throw error;
  }
}

export function startUiHangDiagnostics() {
  if (typeof window === 'undefined' || uiHangDiagnosticsStarted) return;
  uiHangDiagnosticsStarted = true;

  let expectedAt = performance.now() + UI_HANG_INTERVAL_MS;
  let lastReportAt = 0;
  window.setInterval(() => {
    const now = performance.now();
    const driftMs = Math.round(now - expectedAt);
    expectedAt = now + UI_HANG_INTERVAL_MS;
    if (driftMs < UI_HANG_WARN_DRIFT_MS) return;
    if (now - lastReportAt < UI_HANG_REPORT_THROTTLE_MS) return;
    lastReportAt = now;
    logHangDiagnostic('warn', 'UI event loop blocked', {
      driftMs,
      thresholdMs: UI_HANG_WARN_DRIFT_MS,
      visibility: document.visibilityState,
    });
  }, UI_HANG_INTERVAL_MS);
}
