/**
 * @param {{
 *   itemCount: number;
 *   itemHeight: number;
 *   scrollTop: number;
 *   viewportHeight: number;
 *   overscanViewports: number;
 * }} input
 * @returns {{ start: number; end: number }}
 */
export function calculateVirtualWindow({ itemCount, itemHeight, scrollTop, viewportHeight, overscanViewports }) {
  const count = Math.max(0, Math.floor(itemCount));
  if (count === 0 || itemHeight <= 0 || viewportHeight <= 0) return { start: 0, end: 0 };

  const totalHeight = count * itemHeight;
  const maximumScrollTop = Math.max(0, totalHeight - viewportHeight);
  const clampedScrollTop = Math.min(maximumScrollTop, Math.max(0, scrollTop));
  const overscan = Math.max(0, overscanViewports) * viewportHeight;

  return {
    start: Math.max(0, Math.floor((clampedScrollTop - overscan) / itemHeight)),
    end: Math.min(count, Math.ceil((clampedScrollTop + viewportHeight + overscan) / itemHeight)),
  };
}

/**
 * @param {number} scrollTop
 * @param {number} previousItemHeight
 * @param {number} nextItemHeight
 * @returns {number}
 */
export function preserveLogicalScrollTop(scrollTop, previousItemHeight, nextItemHeight) {
  if (previousItemHeight <= 0 || nextItemHeight <= 0) return Math.max(0, scrollTop);
  return Math.max(0, scrollTop) / previousItemHeight * nextItemHeight;
}
