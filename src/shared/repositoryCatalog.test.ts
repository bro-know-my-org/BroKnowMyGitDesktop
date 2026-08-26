import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { repositoryCatalogCapacity } from "./repositoryCatalog";

test("browser and native repository catalog capacities stay synchronized", async () => {
  const persistenceSource = await readFile(
    new URL("../../src-tauri/src/persistence.rs", import.meta.url),
    "utf8",
  );
  const nativeCapacity = persistenceSource.match(
    /pub\(crate\) const MAX_REPOSITORIES: usize = (\d+);/u,
  );

  assert.ok(nativeCapacity, "native repository capacity constant is present");
  assert.equal(Number(nativeCapacity[1]), repositoryCatalogCapacity);
});
