import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const sourceFiles = [
  "src/App.jsx",
  "src/components/PlaybackColumn.jsx",
  "src/components/ParameterWorkspace.jsx",
  "src/components/OutputColumn.jsx",
  "src/components/FeatureDrawers.jsx",
];

test("prototype keeps the current desktop scope visible", async () => {
  const source = (await Promise.all(sourceFiles.map((file) => readFile(file, "utf8")))).join("\n");
  for (const required of ["播放池", "视频处理", "普通声音", "最终效果窗口", "随机插话", "固定话术"]) {
    assert.match(source, new RegExp(required), `missing required desktop capability: ${required}`);
  }
  for (const forbidden of ["实时话术幻化", "speech-to-speech", "RTMP", "OBS", "检测规避"]) {
    assert.doesNotMatch(source, new RegExp(forbidden), `out-of-scope capability leaked into UI: ${forbidden}`);
  }
});

test("desktop layout preserves three-column baseline and responsive stacking", async () => {
  const css = await readFile("src/styles.css", "utf8");
  assert.match(css, /320px\s+minmax\(0,\s*1fr\)\s+272px/);
  assert.match(css, /@media\s*\(max-width:\s*1199px\)/);
});

