const numericIdentifier = "(?:0|[1-9]\\d*)";
const stableVersionSource = `${numericIdentifier}\\.${numericIdentifier}\\.${numericIdentifier}`;
const stableTagPattern = new RegExp(`^v(${stableVersionSource})$`);
const stableVersionPattern = new RegExp(`^v?(${stableVersionSource})$`);
const maxReleaseVersionComponent = 65535n;
const cargoPackageVersionPattern = /^(\[package\]\r?\nname[ \t]*=[ \t]*"broknowmygitdesktop"\r?\nversion[ \t]*=[ \t]*)"([^"\r\n]+)"([ \t]*(?:#[^\r\n]*)?)(?=\r?\n|$)/;

export function parseReleaseTag(tag) {
  if (typeof tag !== "string" || tag.trim() !== tag) return null;
  const version = stableTagPattern.exec(tag)?.[1] ?? null;
  return version && hasValidCargoComponents(version) ? version : null;
}

export function parseVersionInput(input) {
  if (typeof input !== "string" || input.trim() !== input) return null;
  const version = stableVersionPattern.exec(input)?.[1] ?? null;
  return version && hasValidCargoComponents(version) ? version : null;
}

function hasValidCargoComponents(version) {
  return version.split(".").every((component) => BigInt(component) <= maxReleaseVersionComponent);
}

export function setCargoPackageVersion(cargoToml, version) {
  if (parseVersionInput(version) !== version) {
    throw new Error(`Invalid normalized stable version: ${version}`);
  }

  const match = cargoPackageVersionPattern.exec(cargoToml);
  if (!match) {
    throw new Error("Could not update the fixed [package] version in src-tauri/Cargo.toml.");
  }

  const updatedPackage = `${match[1]}"${version}"${match[3]}`;
  return updatedPackage + cargoToml.slice(match[0].length);
}
