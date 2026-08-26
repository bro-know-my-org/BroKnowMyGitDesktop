type PerformanceCategory = "startup" | "command";
type PerformanceStatus = "success" | "error";

interface PerformanceSample {
  category: PerformanceCategory;
  name: string;
  startTime: number;
  duration: number;
  status?: PerformanceStatus;
}

interface CommandSummary {
  command: string;
  count: number;
  totalMs: number;
  averageMs: number;
  maximumMs: number;
  errors: number;
}

interface PerformanceReport {
  context: {
    visibilityState: DocumentVisibilityState | "unavailable";
    focused: boolean;
  };
  startup: Array<{ phase: string; startMs: number; durationMs: number }>;
  commands: Array<{ command: string; startMs: number; durationMs: number; status?: PerformanceStatus }>;
  commandSummary: Array<{
    command: string;
    count: number;
    totalMs: number;
    averageMs: number;
    maximumMs: number;
    errors: number;
  }>;
}

const markPrefix = "bkm:";
const maximumStartupSamples = 100;
const maximumCommandSamples = 500;
const startupSamples: PerformanceSample[] = [];
const commandSamples: PerformanceSample[] = [];

function localPerformanceEnabled(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem("bkm:performance") === "true";
  } catch {
    return false;
  }
}

export const performanceProfilingEnabled =
  typeof performance !== "undefined"
  && typeof performance.now === "function"
  && typeof performance.mark === "function"
  && typeof performance.measure === "function"
  && typeof performance.getEntriesByName === "function"
  && typeof performance.clearMarks === "function"
  && typeof performance.clearMeasures === "function"
  && (import.meta.env?.DEV || import.meta.env?.VITE_BKM_PERF === "true" || localPerformanceEnabled());

if (performanceProfilingEnabled && performance.getEntriesByName(`${markPrefix}bootstrap`, "mark").length === 0) {
  try {
    // Runtime opt-in cannot inject the pre-bundle bootstrap script. Navigation
    // start is the closest stable anchor and keeps Release diagnostics useful.
    const bootstrapMark = `${markPrefix}bootstrap`;
    performance.mark(bootstrapMark, { startTime: 0 });
    const [entry] = performance.getEntriesByName(bootstrapMark, "mark");
    if (!entry || entry.startTime !== 0) performance.clearMarks(bootstrapMark);
  } catch {
    // Without backdated marks, omit navigation-based startup intervals rather
    // than presenting a module-evaluation timestamp as a comparable anchor.
  }
}

function remember(sample: PerformanceSample): void {
  const target = sample.category === "startup" ? startupSamples : commandSamples;
  const maximum = sample.category === "startup" ? maximumStartupSamples : maximumCommandSamples;
  target.push(sample);
  if (target.length > maximum) target.splice(0, target.length - maximum);
}

export function markPerformance(name: string): void {
  if (!performanceProfilingEnabled || typeof performance.mark !== "function") return;
  try {
    const mark = `${markPrefix}${name}`;
    performance.clearMarks(mark);
    performance.mark(mark);
  } catch {
    // Diagnostics must never prevent the application from starting.
  }
}

export function measurePerformance(name: string, startMark: string, endMark: string): void {
  if (!performanceProfilingEnabled || typeof performance.measure !== "function") return;
  try {
    const measure = `${markPrefix}${name}`;
    const entry = performance.measure(
      measure,
      `${markPrefix}${startMark}`,
      `${markPrefix}${endMark}`,
    );
    remember({
      category: "startup",
      name,
      startTime: entry.startTime,
      duration: entry.duration,
    });
    performance.clearMeasures(measure);
  } catch {
    // A missing bootstrap mark should not affect application startup.
  }
}

export async function measureCommand<T>(command: string, operation: () => Promise<T>): Promise<T> {
  if (!performanceProfilingEnabled) return operation();
  const startTime = performance.now();
  try {
    const result = await operation();
    remember({
      category: "command",
      name: command,
      startTime,
      duration: performance.now() - startTime,
      status: "success",
    });
    return result;
  } catch (error) {
    remember({
      category: "command",
      name: command,
      startTime,
      duration: performance.now() - startTime,
      status: "error",
    });
    throw error;
  }
}

function commandSummaries(): CommandSummary[] {
  const summaries = new Map<string, CommandSummary>();
  for (const sample of commandSamples) {
    const current = summaries.get(sample.name) ?? {
      command: sample.name,
      count: 0,
      totalMs: 0,
      averageMs: 0,
      maximumMs: 0,
      errors: 0,
    };
    current.count += 1;
    current.totalMs += sample.duration;
    current.maximumMs = Math.max(current.maximumMs, sample.duration);
    if (sample.status === "error") current.errors += 1;
    current.averageMs = current.totalMs / current.count;
    summaries.set(sample.name, current);
  }
  return [...summaries.values()].sort((left, right) => right.totalMs - left.totalMs);
}

export function performanceSnapshot(): PerformanceReport {
  const context = {
    visibilityState: typeof document === "undefined" ? "unavailable" as const : document.visibilityState,
    focused: typeof document !== "undefined" && document.hasFocus(),
  };
  if (!performanceProfilingEnabled) return { context, startup: [], commands: [], commandSummary: [] };
  const startupRows = startupSamples
    .map((sample) => ({
      phase: sample.name,
      startMs: Number(sample.startTime.toFixed(1)),
      durationMs: Number(sample.duration.toFixed(1)),
    }));
  const commandRows = commandSamples
    .map((sample) => ({
      command: sample.name,
      startMs: Number(sample.startTime.toFixed(1)),
      durationMs: Number(sample.duration.toFixed(1)),
      status: sample.status,
    }));
  const commandSummaryRows = commandSummaries().map((summary) => ({
    command: summary.command,
    count: summary.count,
    totalMs: Number(summary.totalMs.toFixed(1)),
    averageMs: Number(summary.averageMs.toFixed(1)),
    maximumMs: Number(summary.maximumMs.toFixed(1)),
    errors: summary.errors,
  }));

  return { context, startup: startupRows, commands: commandRows, commandSummary: commandSummaryRows };
}

export function reportPerformance(): void {
  if (!performanceProfilingEnabled) return;
  const report = performanceSnapshot();

  console.groupCollapsed("[BKM performance] startup and command timings");
  console.info("Capture context", report.context);
  if (report.context.visibilityState !== "visible" || !report.context.focused) {
    console.warn("The WebView was not focused and visible when this report was printed; frame timings may be throttled.");
  }
  if (report.startup.length) console.table(report.startup);
  if (report.commands.length) console.table(report.commands);
  if (report.commandSummary.length) console.table(report.commandSummary);
  console.info("Run window.__BKM_PERFORMANCE__.report() to print the latest samples again.");
  console.groupEnd();
}

export function completeStartupPerformance(onInteractiveFrame?: () => void): void {
  if (performanceProfilingEnabled) {
    markPerformance("workspace:startup-complete");
    measurePerformance("workspace startup", "workspace:mounted", "workspace:startup-complete");
  }
  let completed = false;
  const complete = (afterFrame: boolean) => {
    if (completed) return;
    completed = true;
    if (performanceProfilingEnabled) {
      const mark = afterFrame ? "workspace:interactive-frame" : "workspace:deferred-startup";
      const label = afterFrame ? "Bootstrap to interactive frame" : "Bootstrap to deferred startup";
      markPerformance(mark);
      measurePerformance(label, "bootstrap", mark);
      reportPerformance();
    }
    onInteractiveFrame?.();
  };
  const fallback = setTimeout(() => complete(false), 1_000);
  if (typeof requestAnimationFrame !== "function") return;
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      clearTimeout(fallback);
      complete(true);
    });
  });
}

if (typeof window !== "undefined") {
  window.__BKM_PERFORMANCE__ = {
    enabled: performanceProfilingEnabled,
    report: reportPerformance,
    snapshot: performanceSnapshot,
  };
}

declare global {
  interface Window {
    __BKM_PERFORMANCE__: {
      enabled: boolean;
      report: () => void;
      snapshot: () => PerformanceReport;
    };
  }
}
