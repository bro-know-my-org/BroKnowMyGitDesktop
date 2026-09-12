<script setup lang="ts">
import hljs from "highlight.js/lib/common";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { NScrollbar } from "naive-ui";
import { diffLineWrapping } from "../../../shared/diffPreferences";
import { resolvedDiffLineHeight } from "../../../shared/fonts";

const props = defineProps<{
  lines: string[];
  lineClass: (line: string) => string;
  filePath: string;
  resetKey: string;
  label: string;
}>();

defineSlots<{
  "line-action"?(props: { line: string; index: number }): unknown;
}>();

const diffLineOverscan = 24;
const maximumMeasuredColumns = 10_000;
const maximumHighlightedHunkCharacters = 100_000;
const maximumHighlightCacheCharacters = 500_000;
const maximumHighlightCacheEntries = 128;
const wrappedLineChunkSize = 500;
const scrollbar = ref<InstanceType<typeof NScrollbar> | null>(null);
const scroller = computed(() => scrollbar.value?.scrollbarInstRef?.containerRef ?? null);
const wrappedLinesContainer = ref<HTMLElement | null>(null);
const wrappedRenderSentinel = ref<HTMLElement | null>(null);
const visibleStart = ref(0);
const visibleEnd = ref(0);
const wrappedRenderedLineCount = ref(wrappedLineChunkSize);
const wrappedHighlightHtml = ref<ReadonlyMap<number, string | null>>(new Map());
let viewportAnimationFrame: number | null = null;
let viewportResizeObserver: ResizeObserver | null = null;
let wrappedHighlightObserver: IntersectionObserver | null = null;
let wrappedRenderObserver: IntersectionObserver | null = null;
let wrappedHighlightObserverGeneration = 0;
let wrappedRenderObserverGeneration = 0;
let wrappedLineElements: HTMLElement[] = [];
let viewerMounted = false;
let highlightCacheCharacters = 0;

interface HighlightedHunk {
  characterCount: number;
  lines: Map<number, string>;
}

const highlightCache = new Map<string, HighlightedHunk | null>();

interface DiffLinePresentation {
  index: number;
  line: string;
  oldLineNumber: number | null;
  newLineNumber: number | null;
  prefix: string;
  content: string;
  highlightable: boolean;
  hunkIndex: number | null;
}

const languageByExtension: Record<string, string> = {
  bash: "bash",
  c: "c",
  cc: "cpp",
  cjs: "javascript",
  cpp: "cpp",
  cs: "csharp",
  css: "css",
  cxx: "cpp",
  go: "go",
  h: "c",
  hpp: "cpp",
  htm: "xml",
  html: "xml",
  ini: "ini",
  java: "java",
  js: "javascript",
  json: "json",
  jsx: "javascript",
  kt: "kotlin",
  kts: "kotlin",
  less: "less",
  lua: "lua",
  md: "markdown",
  mjs: "javascript",
  php: "php",
  plist: "xml",
  py: "python",
  rb: "ruby",
  rs: "rust",
  sass: "scss",
  scss: "scss",
  sh: "bash",
  sql: "sql",
  swift: "swift",
  toml: "ini",
  ts: "typescript",
  tsx: "typescript",
  vue: "xml",
  xml: "xml",
  yaml: "yaml",
  yml: "yaml",
  zsh: "bash",
};

const languageByFilename: Record<string, string> = {
  "dockerfile": "dockerfile",
  "gemfile": "ruby",
  "makefile": "makefile",
  "rakefile": "ruby",
};

function resolveLanguage(filePath: string): string | null {
  const filename = filePath.split(/[\\/]/u).pop()?.toLocaleLowerCase() || "";
  const namedLanguage = languageByFilename[filename];
  if (namedLanguage && hljs.getLanguage(namedLanguage)) return namedLanguage;
  const extension = filename.includes(".") ? filename.slice(filename.lastIndexOf(".") + 1) : "";
  const extensionLanguage = languageByExtension[extension];
  return extensionLanguage && hljs.getLanguage(extensionLanguage) ? extensionLanguage : null;
}

function presentDiffLines(lines: string[]): DiffLinePresentation[] {
  let oldLineNumber = 0;
  let newLineNumber = 0;
  let insideHunk = false;
  let hunkIndex = -1;

  return lines.map((line, index) => {
    if (line.startsWith("diff --git ")) insideHunk = false;
    const hunkMatch = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/u.exec(line);
    if (hunkMatch) {
      oldLineNumber = Number(hunkMatch[1]);
      newLineNumber = Number(hunkMatch[2]);
      insideHunk = true;
      hunkIndex += 1;
      return { index, line, oldLineNumber: null, newLineNumber: null, prefix: "", content: line, highlightable: false, hunkIndex: null };
    }

    if (!insideHunk || line.startsWith("\\ No newline at end of file")) {
      return { index, line, oldLineNumber: null, newLineNumber: null, prefix: "", content: line, highlightable: false, hunkIndex: null };
    }

    const prefix = line[0] || " ";
    const content = line.length ? line.slice(1) : "";
    if (prefix === "+") {
      const presentation = { index, line, oldLineNumber: null, newLineNumber, prefix, content, highlightable: true, hunkIndex };
      newLineNumber += 1;
      return presentation;
    }
    if (prefix === "-") {
      const presentation = { index, line, oldLineNumber, newLineNumber: null, prefix, content, highlightable: true, hunkIndex };
      oldLineNumber += 1;
      return presentation;
    }
    if (prefix === " ") {
      const presentation = { index, line, oldLineNumber, newLineNumber, prefix, content, highlightable: true, hunkIndex };
      oldLineNumber += 1;
      newLineNumber += 1;
      return presentation;
    }

    return { index, line, oldLineNumber: null, newLineNumber: null, prefix: "", content: line, highlightable: false, hunkIndex: null };
  });
}

function splitHighlightedHtml(highlighted: string): string[] {
  const lines = [""];
  const openSpans: string[] = [];
  const tokenPattern = /<span class="[^"]+">|<\/span>|\n/gu;
  let sourceIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = tokenPattern.exec(highlighted)) !== null) {
    lines[lines.length - 1] += highlighted.slice(sourceIndex, match.index);
    const token = match[0];
    if (token === "\n") {
      lines[lines.length - 1] += "</span>".repeat(openSpans.length);
      lines.push(openSpans.join(""));
    } else if (token === "</span>") {
      lines[lines.length - 1] += token;
      openSpans.pop();
    } else {
      lines[lines.length - 1] += token;
      openSpans.push(token);
    }
    sourceIndex = match.index + token.length;
  }
  lines[lines.length - 1] += highlighted.slice(sourceIndex);
  return lines;
}

function cacheHighlightedHunk(cacheKey: string, highlightedHunk: HighlightedHunk | null): void {
  if (highlightedHunk && highlightedHunk.characterCount > maximumHighlightCacheCharacters) return;
  while (
    highlightCache.size >= maximumHighlightCacheEntries
    || highlightCacheCharacters + (highlightedHunk?.characterCount || 0) > maximumHighlightCacheCharacters
  ) {
    const oldestKey = highlightCache.keys().next().value as string | undefined;
    if (oldestKey === undefined) break;
    const oldest = highlightCache.get(oldestKey);
    if (oldest) highlightCacheCharacters -= oldest.characterCount;
    highlightCache.delete(oldestKey);
  }
  highlightCache.set(cacheKey, highlightedHunk);
  highlightCacheCharacters += highlightedHunk?.characterCount || 0;
}

function highlightedHunk(language: string, hunkIndex: number, side: "old" | "new"): HighlightedHunk | null {
  const cacheKey = `${language}\u0000${hunkIndex}\u0000${side}`;
  if (highlightCache.has(cacheKey)) {
    const cached = highlightCache.get(cacheKey) ?? null;
    highlightCache.delete(cacheKey);
    highlightCache.set(cacheKey, cached);
    return cached;
  }

  const hunkLines = (hunkLinesByIndex.value.get(hunkIndex) || []).filter((line) => (
    line.highlightable
    && (line.prefix === " " || (side === "old" ? line.prefix === "-" : line.prefix === "+"))
  ));
  const sourceLength = hunkLines.reduce((length, line) => length + line.content.length + 1, 0);
  if (
    sourceLength > maximumHighlightedHunkCharacters
    || hunkLines.some((line) => line.content.length > maximumMeasuredColumns)
  ) {
    cacheHighlightedHunk(cacheKey, null);
    return null;
  }

  const highlighted = hljs.highlight(hunkLines.map((line) => line.content).join("\n"), {
    language,
    ignoreIllegals: true,
  }).value;
  if (highlighted.length > maximumHighlightCacheCharacters) {
    cacheHighlightedHunk(cacheKey, null);
    return null;
  }
  const highlightedLines = splitHighlightedHtml(highlighted);
  const lines = new Map<number, string>();
  hunkLines.forEach((line, index) => lines.set(line.index, highlightedLines[index] || ""));
  const result = { characterCount: highlighted.length, lines };
  cacheHighlightedHunk(cacheKey, result);
  return result;
}

function highlightedCode(entry: DiffLinePresentation, language: string | null): string | null {
  if (!language || entry.hunkIndex === null) return null;
  const side = entry.prefix === "-" ? "old" : "new";
  return highlightedHunk(language, entry.hunkIndex, side)?.lines.get(entry.index) ?? null;
}

function clearHighlightCache(): void {
  highlightCache.clear();
  highlightCacheCharacters = 0;
}

const language = computed(() => resolveLanguage(props.filePath));
const presentedLines = computed(() => presentDiffLines(props.lines));
const hunkLinesByIndex = computed(() => {
  const hunks = new Map<number, DiffLinePresentation[]>();
  for (const line of presentedLines.value) {
    if (line.hunkIndex === null) continue;
    const lines = hunks.get(line.hunkIndex) || [];
    lines.push(line);
    hunks.set(line.hunkIndex, lines);
  }
  return hunks;
});

const visibleLines = computed(() => presentedLines.value
  .slice(visibleStart.value, visibleEnd.value)
  .map((entry) => ({
    ...entry,
    highlightedContent: entry.highlightable ? highlightedCode(entry, language.value) : null,
  })));
const wrappedPresentedLines = computed(() => presentedLines.value.slice(0, wrappedRenderedLineCount.value));
const canRenderMoreWrappedLines = computed(() => wrappedRenderedLineCount.value < presentedLines.value.length);

const lineNumberColumns = computed(() => {
  const maximumLineNumber = presentedLines.value.reduce((maximum, line) => Math.max(
    maximum,
    line.oldLineNumber || 0,
    line.newLineNumber || 0,
  ), 0);
  return Math.max(3, String(maximumLineNumber).length);
});

function estimatedLineColumns(line: string): number {
  let columns = 0;
  for (const character of line) {
    if (character === "\t") {
      columns += 8 - (columns % 8);
    } else if (character.codePointAt(0)! > 0x7f) {
      // Monospace fallbacks commonly render CJK and emoji at two cells. Treat
      // all non-ASCII characters conservatively so the diff canvas never ends
      // before the browser's actual glyph run.
      columns += 2;
    } else {
      columns += 1;
    }
    if (columns >= maximumMeasuredColumns) return maximumMeasuredColumns;
  }
  return columns;
}

const maximumLineColumns = computed(() => {
  let maximum = 0;
  for (const line of props.lines) {
    maximum = Math.max(maximum, estimatedLineColumns(line));
    if (maximum >= maximumMeasuredColumns) break;
  }
  return maximum;
});

const spacerStyle = computed(() => ({
  height: `${props.lines.length * resolvedDiffLineHeight.value}px`,
  minWidth: `max(100%, calc(${maximumLineColumns.value + (lineNumberColumns.value * 2) + 8}ch + 28px))`,
}));

const lineNumberStyle = computed(() => ({
  width: `${(lineNumberColumns.value + 2) * 2}ch`,
}));

function updateVisibleLines(): void {
  viewportAnimationFrame = null;
  const element = scroller.value;
  if (!element) return;
  const firstVisible = Math.floor(element.scrollTop / resolvedDiffLineHeight.value);
  const visibleCount = Math.ceil(element.clientHeight / resolvedDiffLineHeight.value);
  visibleStart.value = Math.max(0, firstVisible - diffLineOverscan);
  visibleEnd.value = Math.min(props.lines.length, firstVisible + visibleCount + diffLineOverscan);
}

function highlightWrappedIndexes(indexes: Iterable<number>): void {
  const highlighted = new Map(wrappedHighlightHtml.value);
  let changed = false;
  for (const index of indexes) {
    const entry = presentedLines.value[index];
    if (!entry || highlighted.has(index)) continue;
    highlighted.set(index, entry.highlightable ? highlightedCode(entry, language.value) : null);
    changed = true;
  }
  if (changed) wrappedHighlightHtml.value = highlighted;
}

function updateFallbackWrappedHighlights(): void {
  viewportAnimationFrame = null;
  const element = scroller.value;
  if (!element || typeof IntersectionObserver !== "undefined") return;
  const overscan = diffLineOverscan * resolvedDiffLineHeight.value;
  const viewportTop = Math.max(0, element.scrollTop - overscan);
  const viewportBottom = element.scrollTop + element.clientHeight + overscan;
  const indexes: number[] = [];
  let low = 0;
  let high = wrappedLineElements.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (wrappedLineElements[middle].offsetTop < viewportTop) low = middle + 1;
    else high = middle;
  }
  const firstCandidate = Math.max(0, low - 1);
  for (let position = firstCandidate; position < wrappedLineElements.length; position += 1) {
    const line = wrappedLineElements[position];
    if (line.offsetTop > viewportBottom) break;
    if (line.offsetTop + line.offsetHeight < viewportTop) continue;
    const index = Number(line.dataset.diffLineIndex);
    if (Number.isInteger(index)) indexes.push(index);
  }
  highlightWrappedIndexes(indexes);
  if (canRenderMoreWrappedLines.value && element.scrollTop + element.clientHeight >= element.scrollHeight - overscan) {
    renderMoreWrappedLines();
  }
}

function scheduleVisibleLineUpdate(): void {
  if (!viewerMounted || viewportAnimationFrame !== null) return;
  if (!diffLineWrapping.value) viewportAnimationFrame = window.requestAnimationFrame(updateVisibleLines);
  else if (typeof IntersectionObserver === "undefined") {
    viewportAnimationFrame = window.requestAnimationFrame(updateFallbackWrappedHighlights);
  }
}

function disconnectWrappedHighlightObserver(): void {
  wrappedHighlightObserverGeneration += 1;
  wrappedHighlightObserver?.disconnect();
  wrappedHighlightObserver = null;
}

function disconnectWrappedRenderObserver(): void {
  wrappedRenderObserverGeneration += 1;
  wrappedRenderObserver?.disconnect();
  wrappedRenderObserver = null;
}

function observeWrappedRenderSentinel(): void {
  disconnectWrappedRenderObserver();
  const element = scroller.value;
  const sentinel = wrappedRenderSentinel.value;
  if (
    !viewerMounted
    || !diffLineWrapping.value
    || !canRenderMoreWrappedLines.value
    || !element
    || !sentinel
    || typeof IntersectionObserver === "undefined"
  ) return;
  const observerGeneration = wrappedRenderObserverGeneration;
  wrappedRenderObserver = new IntersectionObserver((entries) => {
    if (
      observerGeneration === wrappedRenderObserverGeneration
      && entries.some((entry) => entry.isIntersecting)
    ) renderMoreWrappedLines();
  }, {
    root: element,
    rootMargin: "800px 0px",
  });
  wrappedRenderObserver.observe(sentinel);
}

function renderMoreWrappedLines(): void {
  if (!viewerMounted || !diffLineWrapping.value || !canRenderMoreWrappedLines.value) return;
  wrappedRenderedLineCount.value = Math.min(
    presentedLines.value.length,
    wrappedRenderedLineCount.value + wrappedLineChunkSize,
  );
  const observerGeneration = wrappedRenderObserverGeneration;
  void nextTick(() => {
    if (observerGeneration !== wrappedRenderObserverGeneration) return;
    observeWrappedHighlightTargets();
    observeWrappedRenderSentinel();
  });
}

function observeWrappedHighlightTargets(recreateObserver = false): void {
  const element = scroller.value;
  const linesContainer = wrappedLinesContainer.value;
  if (!viewerMounted || !diffLineWrapping.value || !element || !linesContainer) return;
  const previousLineCount = wrappedLineElements.length;
  const renderedLineCount = wrappedPresentedLines.value.length;
  for (let index = previousLineCount; index < renderedLineCount; index += 1) {
    const line = linesContainer.children.item(index);
    if (line instanceof HTMLElement) wrappedLineElements.push(line);
  }
  if (typeof IntersectionObserver === "undefined") {
    updateFallbackWrappedHighlights();
    return;
  }

  if (recreateObserver) disconnectWrappedHighlightObserver();
  if (!wrappedHighlightObserver) {
    const observerGeneration = wrappedHighlightObserverGeneration;
    wrappedHighlightObserver = new IntersectionObserver((entries, observer) => {
      if (observerGeneration !== wrappedHighlightObserverGeneration) return;
      const indexes: number[] = [];
      for (const observed of entries) {
        if (!observed.isIntersecting) continue;
        const index = Number((observed.target as HTMLElement).dataset.diffLineIndex);
        if (Number.isInteger(index)) indexes.push(index);
        observer.unobserve(observed.target);
      }
      highlightWrappedIndexes(indexes);
    }, {
      root: element,
      rootMargin: `${diffLineOverscan * resolvedDiffLineHeight.value}px 0px`,
    });
  }

  const firstTarget = recreateObserver ? 0 : previousLineCount;
  wrappedLineElements.slice(firstTarget).forEach((line) => {
    const index = Number(line.dataset.diffLineIndex);
    if (Number.isInteger(index) && !wrappedHighlightHtml.value.has(index)) {
      wrappedHighlightObserver?.observe(line);
    }
  });
}

function resetWrappedHighlights(resetRenderedLines = true): void {
  wrappedHighlightHtml.value = new Map();
  wrappedLineElements = [];
  disconnectWrappedHighlightObserver();
  disconnectWrappedRenderObserver();
  if (resetRenderedLines) wrappedRenderedLineCount.value = wrappedLineChunkSize;
  if (diffLineWrapping.value) {
    const highlightGeneration = wrappedHighlightObserverGeneration;
    const renderGeneration = wrappedRenderObserverGeneration;
    void nextTick(() => {
      if (
        highlightGeneration !== wrappedHighlightObserverGeneration
        || renderGeneration !== wrappedRenderObserverGeneration
      ) return;
      observeWrappedHighlightTargets();
      observeWrappedRenderSentinel();
    });
  }
}

watch(() => props.lines.length, () => {
  void nextTick(scheduleVisibleLineUpdate);
});

watch(resolvedDiffLineHeight, (lineHeight, previousLineHeight) => {
  const element = scroller.value;
  const logicalScrollPosition = element && !diffLineWrapping.value && previousLineHeight > 0
    ? element.scrollTop / previousLineHeight
    : null;
  void nextTick(() => {
    if (element && logicalScrollPosition !== null) {
      element.scrollTop = logicalScrollPosition * lineHeight;
    }
    if (diffLineWrapping.value) {
      observeWrappedHighlightTargets(true);
    }
    else scheduleVisibleLineUpdate();
  });
});

watch(() => props.lines, () => {
  clearHighlightCache();
  resetWrappedHighlights(false);
});

watch(language, () => {
  clearHighlightCache();
  wrappedHighlightHtml.value = new Map();
  disconnectWrappedHighlightObserver();
  const observerGeneration = wrappedHighlightObserverGeneration;
  if (diffLineWrapping.value) {
    void nextTick(() => {
      if (observerGeneration === wrappedHighlightObserverGeneration) observeWrappedHighlightTargets(true);
    });
  }
});

watch(diffLineWrapping, (wrapping) => {
  const element = scroller.value;
  if (element) element.scrollTop = 0;
  if (wrapping) {
    resetWrappedHighlights();
  } else {
    wrappedHighlightHtml.value = new Map();
    wrappedLineElements = [];
    disconnectWrappedHighlightObserver();
    disconnectWrappedRenderObserver();
    if (viewportAnimationFrame !== null) {
      window.cancelAnimationFrame(viewportAnimationFrame);
      viewportAnimationFrame = null;
    }
    void nextTick(scheduleVisibleLineUpdate);
  }
});

watch(() => props.resetKey, () => {
  clearHighlightCache();
  resetWrappedHighlights();
  const element = scroller.value;
  if (element) element.scrollTop = 0;
  void nextTick(scheduleVisibleLineUpdate);
});

onMounted(() => {
  viewerMounted = true;
  viewportResizeObserver = new ResizeObserver(scheduleVisibleLineUpdate);
  if (scroller.value) viewportResizeObserver.observe(scroller.value);
  scheduleVisibleLineUpdate();
  if (diffLineWrapping.value) {
    void nextTick(() => {
      observeWrappedHighlightTargets();
      observeWrappedRenderSentinel();
    });
  }
});

onBeforeUnmount(() => {
  viewerMounted = false;
  viewportResizeObserver?.disconnect();
  disconnectWrappedHighlightObserver();
  disconnectWrappedRenderObserver();
  if (viewportAnimationFrame !== null) window.cancelAnimationFrame(viewportAnimationFrame);
});
</script>

<template>
  <NScrollbar
    ref="scrollbar"
    class="diff-output virtual-diff-output"
    :class="{ 'wrapped-diff-output': diffLineWrapping }"
    :x-scrollable="!diffLineWrapping"
    @scroll="scheduleVisibleLineUpdate"
  >
    <div role="region" tabindex="0" :aria-label="label">
    <div v-if="!diffLineWrapping" class="virtual-diff-spacer" :style="spacerStyle">
      <span
        v-for="entry in visibleLines"
        :key="entry.index"
        class="diff-line virtual-diff-line"
        :class="lineClass(entry.line)"
        :style="{ top: `${entry.index * resolvedDiffLineHeight}px` }"
      >
        <span class="diff-line-numbers" :style="lineNumberStyle" aria-hidden="true">
          <span>{{ entry.oldLineNumber ?? '' }}</span>
          <span>{{ entry.newLineNumber ?? '' }}</span>
        </span>
        <span class="virtual-diff-line-text">
          <span v-if="entry.highlightable" class="diff-line-prefix">{{ entry.prefix }}</span>
          <span v-if="entry.highlightedContent !== null" class="diff-line-code" v-html="entry.highlightedContent" />
          <span v-else>{{ entry.content }}</span>
        </span>
        <span v-if="$slots['line-action']" class="virtual-diff-line-action">
          <slot name="line-action" :line="entry.line" :index="entry.index" />
        </span>
      </span>
    </div>
    <div v-else ref="wrappedLinesContainer" class="wrapped-diff-lines">
      <span
        v-for="entry in wrappedPresentedLines"
        :key="entry.index"
        class="diff-line wrapped-diff-line"
        :class="lineClass(entry.line)"
        :data-diff-line-index="entry.index"
      >
        <span class="diff-line-numbers" :style="lineNumberStyle" aria-hidden="true">
          <span>{{ entry.oldLineNumber ?? '' }}</span>
          <span>{{ entry.newLineNumber ?? '' }}</span>
        </span>
        <span class="wrapped-diff-line-text">
          <span v-if="entry.highlightable" class="diff-line-prefix">{{ entry.prefix }}</span>
          <span
            v-if="wrappedHighlightHtml.has(entry.index) && wrappedHighlightHtml.get(entry.index) !== null"
            class="diff-line-code"
            v-html="wrappedHighlightHtml.get(entry.index) || ''"
          />
          <span v-else>{{ entry.content }}</span>
        </span>
        <span v-if="$slots['line-action']" class="virtual-diff-line-action wrapped-diff-line-action">
          <slot name="line-action" :line="entry.line" :index="entry.index" />
        </span>
      </span>
      <span
        v-if="canRenderMoreWrappedLines"
        ref="wrappedRenderSentinel"
        class="wrapped-diff-render-sentinel"
        aria-hidden="true"
      />
    </div>
    </div>
  </NScrollbar>
</template>
