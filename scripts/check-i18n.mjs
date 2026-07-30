import { readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const localesRoot = fileURLToPath(new URL("../src/shared/i18n/locales", import.meta.url));
const frontendSourceRoot = fileURLToPath(new URL("../src", import.meta.url));
const rustSourceRoot = fileURLToPath(new URL("../src-tauri/src", import.meta.url));
const browserApiPath = fileURLToPath(new URL("../src/shared/api/tauri.ts", import.meta.url));
const sourceLocale = "zh-CN";
const placeholderPattern = /\{([A-Za-z][A-Za-z0-9_]*)\}/gu;
const messageKeyPattern = /^(?:common|workspace|repository|accounts|errors|demo)(?:\.[a-z][A-Za-z0-9_]*)+$/u;

async function loadLocale(locale) {
  const localeRoot = `${localesRoot}/${locale}`;
  const files = (await readdir(localeRoot)).filter((file) => file.endsWith(".json")).sort();
  const messages = new Map();

  for (const file of files) {
    const document = JSON.parse(await readFile(`${localeRoot}/${file}`, "utf8"));
    if (!document || Array.isArray(document) || typeof document !== "object") {
      throw new Error(`${locale}/${file} must contain a JSON object.`);
    }

    for (const [key, value] of Object.entries(document)) {
      if (typeof value !== "string") throw new Error(`${locale}/${file}: ${key} must be a string.`);
      if (!messageKeyPattern.test(key)) {
        throw new Error(`${locale}/${file}: ${key} is not a valid namespaced message key.`);
      }
      if (messages.has(key)) {
        throw new Error(`${locale}: duplicate key ${key} in ${messages.get(key).file} and ${file}.`);
      }
      messages.set(key, { file, value });
    }
  }

  return { files, messages };
}

async function sourceFiles(root, extension) {
  const entries = await readdir(root, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const path = `${root}/${entry.name}`;
    if (entry.isDirectory()) files.push(...await sourceFiles(path, extension));
    else if (entry.name.endsWith(extension)) files.push(path);
  }
  return files;
}

function placeholders(message) {
  return [...message.matchAll(placeholderPattern)].map((match) => match[1]).sort().join(",");
}

const localeNames = (await readdir(localesRoot, { withFileTypes: true }))
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .sort();
const source = await loadLocale(sourceLocale);
const failures = [];

for (const locale of localeNames) {
  const catalog = await loadLocale(locale);
  if (catalog.files.join("\n") !== source.files.join("\n")) {
    failures.push(`${locale}: locale files differ from ${sourceLocale}.`);
  }

  for (const [key, sourceEntry] of source.messages) {
    const translated = catalog.messages.get(key);
    if (!translated) {
      failures.push(`${locale}: missing key ${key}.`);
      continue;
    }
    if (placeholders(translated.value) !== placeholders(sourceEntry.value)) {
      failures.push(`${locale}: placeholders for ${key} differ from ${sourceLocale}.`);
    }
  }

  for (const key of catalog.messages.keys()) {
    if (!source.messages.has(key)) failures.push(`${locale}: unexpected key ${key}.`);
  }
}

const requiredErrorCodes = new Set();
for (const file of await sourceFiles(rustSourceRoot, ".rs")) {
  const sourceText = await readFile(file, "utf8");
  for (const match of sourceText.matchAll(/(?:AppError|Self)::(?:new|with_safe_details)\(\s*"([a-z0-9_]+)"/gu)) {
    if (match[1] !== "example") requiredErrorCodes.add(match[1]);
  }
  for (const match of sourceText.matchAll(/\btimeout_code:\s*"([a-z0-9_]+)"/gu)) {
    requiredErrorCodes.add(match[1]);
  }
  for (const match of sourceText.matchAll(/normalize_repository_label\(\s*[^,]+,\s*"([a-z0-9_]+)"/gu)) {
    requiredErrorCodes.add(match[1]);
  }
  if (file.endsWith("/git_core/error.rs")) {
    const classifier = sourceText.match(/fn git_error_from_diagnostic[\s\S]*?\n\}\n\npub\(super\) fn contains_any/u)?.[0] ?? "";
    for (const match of classifier.matchAll(/"((?:git_|not_a_git_repository)[a-z0-9_]*)"/gu)) {
      requiredErrorCodes.add(match[1]);
    }
  }
}
const browserApiSource = await readFile(browserApiPath, "utf8");
for (const match of browserApiSource.matchAll(/\bcode:\s*"([a-z0-9_]+)"/gu)) {
  requiredErrorCodes.add(match[1]);
}
for (const match of browserApiSource.matchAll(/\bbrowserError\(\s*"([a-z0-9_]+)"/gu)) {
  requiredErrorCodes.add(match[1]);
}
for (const file of [
  ...await sourceFiles(frontendSourceRoot, ".ts"),
  ...await sourceFiles(frontendSourceRoot, ".vue"),
]) {
  const sourceText = await readFile(file, "utf8");
  for (const match of sourceText.matchAll(/\bcode:\s*"([a-z0-9_]+)"/gu)) {
    requiredErrorCodes.add(match[1]);
  }
}
for (const code of [...requiredErrorCodes].sort()) {
  if (!source.messages.has(`errors.${code}`)) failures.push(`${sourceLocale}: missing catalog entry errors.${code}.`);
}

if (failures.length) {
  throw new Error(`i18n validation failed:\n- ${failures.join("\n- ")}`);
}

console.log(`i18n validation passed: ${source.messages.size} keys across ${localeNames.length} locales; ${requiredErrorCodes.size} static error codes covered.`);
