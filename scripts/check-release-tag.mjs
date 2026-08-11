import { parseReleaseTag } from "./release-version-utils.mjs";

const tag = process.env.RELEASE_TAG ?? process.argv[2];
const version = parseReleaseTag(tag);

if (!version || version === "0.0.0") {
  console.error("Release tag must use the stable vX.Y.Z format (for example, v0.1.0).");
  process.exit(1);
}

console.log(`Release tag ${tag} is valid.`);
