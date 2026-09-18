#!/usr/bin/env python3

import re
import subprocess
import sys

"""
QuestieTrace release script — bumps the TOC version and creates a git tag.

Usage:
    python release.py <version>          Bump to <version>, commit, and tag
    python release.py <version> --push   Also push the commit and tag

<version> must be a bare semver string without the "v" prefix (the script
adds it), optionally with a "-bN" beta suffix, e.g.:

    python release.py 1.2.0
    python release.py 1.2.0-b1

The CI publish pipeline (.github/workflows/publish.yml) triggers on pushed
"v*" tags, builds the zip via build.py, and creates the GitHub release.
"""

TOC_FILES = ["QuestieTrace-Classic.toc", "QuestieTrace-Camelot.toc"]
VERSION_PATTERN = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-b\d+)?$")


def main():
    args = sys.argv[1:]
    if not args:
        print("Needs new version number provided as argument, e.g. 1.2.0")
        sys.exit(1)

    version = args[0]
    push = "--push" in args[1:]

    if version.startswith("v"):
        print('Please omit the "v" prefix. The script will add it.')
        sys.exit(1)

    if not VERSION_PATTERN.match(version):
        print(f"'{version}' is not a valid version. Expected e.g. 1.2.0 or 1.2.0-b1")
        sys.exit(1)

    tag = "v" + version

    if tag_exists(tag):
        print(f"Tag '{tag}' already exists.")
        sys.exit(1)

    if has_uncommitted_changes():
        print("Working tree has uncommitted changes. Commit or stash them first.")
        sys.exit(1)

    bump_toc_versions(version)

    try:
        subprocess.run(["git", "add"] + TOC_FILES, check=True)
        subprocess.run(["git", "commit", "-m", f"chore: bump version to {tag}"], check=True)
    except subprocess.CalledProcessError:
        subprocess.run(["git", "reset", "HEAD"])
        print("git add/commit failed (nothing to commit, hook rejection, ...). Staged changes unstaged.")
        sys.exit(1)

    try:
        subprocess.run(["git", "tag", "-a", tag, "-m", tag], check=True)
    except subprocess.CalledProcessError:
        subprocess.run(["git", "reset", "--keep", "HEAD~1"])
        print("git tag failed. The version-bump commit was undone; working tree preserved.")
        sys.exit(1)

    print(f"Created commit and tag '{tag}'.")

    if push:
        subprocess.run(["git", "push"], check=True)
        subprocess.run(["git", "push", "origin", tag], check=True)
        print(f"Pushed commit and tag '{tag}'.")
    else:
        print(f"Run 'git push && git push --tags' to publish the release.")


def bump_toc_versions(version):
    for toc_file in TOC_FILES:
        with open(toc_file, "r") as f:
            lines = f.readlines()

        with open(toc_file, "w") as f:
            for line in lines:
                if line.startswith("## Version:"):
                    f.write(f"## Version: {version}\n")
                else:
                    f.write(line)


def tag_exists(tag):
    result = subprocess.run(["git", "tag", "-l", tag], capture_output=True, text=True, check=True)
    return result.stdout.strip() != ""


def has_uncommitted_changes():
    result = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True, check=True)
    return result.stdout.strip() != ""


if __name__ == "__main__":
    main()
