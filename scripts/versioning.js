"use strict";

function parseVersion(version) {
  const match = String(version).trim().match(/^(\d+)\.(\d+)\.(\d+)(?:-(alpha|rc)\.(\d+))?(?:\+.*)?$/);
  if (!match) {
    throw new Error(`Invalid version: ${version}`);
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prereleaseTag: match[4] || null,
    prereleaseNumber: match[5] ? Number(match[5]) : null,
  };
}

function formatVersion(parts) {
  const base = `${parts.major}.${parts.minor}.${parts.patch}`;
  if (!parts.prereleaseTag) return base;
  const num = Number.isInteger(parts.prereleaseNumber) ? parts.prereleaseNumber : 1;
  return `${base}-${parts.prereleaseTag}.${num}`;
}

function bump(current, kind) {
  const next = { ...current };
  if (kind === "major") {
    next.major += 1;
    next.minor = 0;
    next.patch = 0;
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else if (kind === "minor") {
    next.minor += 1;
    next.patch = 0;
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else if (kind === "patch") {
    next.patch += 1;
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else if (kind === "alpha") {
    if (next.prereleaseTag === "alpha") {
      next.prereleaseNumber += 1;
    } else if (next.prereleaseTag === "rc") {
      throw new Error("Cannot bump alpha from an rc prerelease");
    } else {
      next.patch += 1;
      next.prereleaseTag = "alpha";
      next.prereleaseNumber = 1;
    }
  } else if (kind === "rc") {
    if (next.prereleaseTag === "rc") {
      next.prereleaseNumber += 1;
    } else if (next.prereleaseTag === "alpha") {
      next.prereleaseTag = "rc";
      next.prereleaseNumber = 1;
    } else {
      next.patch += 1;
      next.prereleaseTag = "rc";
      next.prereleaseNumber = 1;
    }
  } else if (kind === "stable") {
    next.prereleaseTag = null;
    next.prereleaseNumber = null;
  } else {
    throw new Error(`Unknown bump kind: ${kind}`);
  }
  return next;
}

function detectConventionalCommitBump(commitsText) {
  const text = String(commitsText || "");
  if (/(^|\n)[a-z]+(?:\([^)]+\))?!:/m.test(text) || /BREAKING CHANGE:/m.test(text)) {
    return "major";
  }
  if (/(^|\n)feat(?:\([^)]+\))?:/m.test(text)) {
    return "minor";
  }
  return "patch";
}

function recommendReleaseBump(currentVersion, commitsText) {
  const current = typeof currentVersion === "string" ? parseVersion(currentVersion) : currentVersion;
  if (current.prereleaseTag === "alpha") {
    return { bump: "rc", reason: "current version is an alpha prerelease" };
  }
  if (current.prereleaseTag === "rc") {
    return { bump: "stable", reason: "current version is a release candidate" };
  }
  const detected = detectConventionalCommitBump(commitsText);
  return { bump: detected, reason: `detected ${detected} bump from conventional commits` };
}

module.exports = {
  bump,
  detectConventionalCommitBump,
  formatVersion,
  parseVersion,
  recommendReleaseBump,
};
