import { createApp } from "vue";
import "./styles/app.css";
import Root from "./Root.vue";
import { markPerformance, measurePerformance, performanceProfilingEnabled } from "./shared/performance";

markPerformance("main:execute");
measurePerformance("Bootstrap to main module execution", "bootstrap", "main:execute");
markPerformance("vue:create:start");
const app = createApp(Root);
markPerformance("vue:create:end");
measurePerformance("Vue application creation", "vue:create:start", "vue:create:end");
markPerformance("vue:mount:start");
app.mount("#app");
markPerformance("vue:mount:end");
measurePerformance("Vue root mount", "vue:mount:start", "vue:mount:end");
if (performanceProfilingEnabled) {
  let firstFrameRecorded = false;
  const recordFirstFrame = (afterFrame: boolean) => {
    if (firstFrameRecorded) return;
    firstFrameRecorded = true;
    const mark = afterFrame ? "vue:first-frame" : "vue:deferred-startup";
    const label = afterFrame ? "Bootstrap to first Vue frame" : "Bootstrap to deferred startup";
    markPerformance(mark);
    measurePerformance(label, "bootstrap", mark);
  };
  const firstFrameFallback = setTimeout(() => recordFirstFrame(false), 1_000);
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        clearTimeout(firstFrameFallback);
        recordFirstFrame(true);
      });
    });
  }
}
