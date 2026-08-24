export interface VirtualWindowInput {
  itemCount: number;
  itemHeight: number;
  scrollTop: number;
  viewportHeight: number;
  overscanViewports: number;
}

export interface VirtualWindow {
  start: number;
  end: number;
}

export function calculateVirtualWindow(input: VirtualWindowInput): VirtualWindow;
export function preserveLogicalScrollTop(scrollTop: number, previousItemHeight: number, nextItemHeight: number): number;
