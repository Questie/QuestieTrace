#!/usr/bin/env python3

import os
import shutil
import subprocess
import sys
import fileinput
import re

"""
QuestieTrace build script — packages addon and generates release metadata.

Usage:
    python build.py -r              Release build (no commit hash in version dir)
    python build.py                 Dev build (includes commit hash)
    python build.py -v <version>    Override version string
"""

addonDir = "QuestieTrace"


def main():
    isReleaseBuild = False
    versionOverride = ""

    if len(sys.argv) > 1:
        ver = False
        for arg in sys.argv[1:]:
            if ver:
                versionOverride = arg
                ver = False
            elif arg in ["-r", "--release"]:
                isReleaseBuild = True
                print("Creating a release build")
            elif arg in ["-v", "--version"]:
                ver = True

    release_dir = get_version_dir(isReleaseBuild, versionOverride)

    if os.path.isdir("releases/%s" % release_dir):
        print("Warning: Folder already exists, removing!")
        shutil.rmtree("releases/%s" % release_dir)

    release_folder_path = "releases/%s" % release_dir
    release_addon_folder_path = release_folder_path + ("/%s" % addonDir)

    copy_content_to(release_addon_folder_path)

    if versionOverride != "":
        for toc_file in ["QuestieTrace-Classic.toc", "QuestieTrace-Camelot.toc"]:
            toc_path = release_addon_folder_path + "/" + toc_file
            with fileinput.FileInput(toc_path, inplace=True) as file:
                for line in file:
                    if line.startswith("## Version"):
                        print("## Version: " + versionOverride)
                    else:
                        print(line, end="")

    zip_name = "%s-%s" % (addonDir, release_dir)
    zip_release_folder(zip_name, release_dir, addonDir)

    # Parse interface versions and map to flavors
    interface_versions = get_interface_versions()

    # Map leading digit to flavor: 1=classic, 2=bcc, 3=wrath, 5=mists, 16=forever
    flavor_map = {"1": "classic", "2": "bcc", "3": "wrath", "5": "mists", "16": "forever"}

    metadata_list = []
    for version in interface_versions:
        version = version.strip()
        # Handle multi-digit interface versions (e.g., "16001" for forever)
        flavor_digit = version[:2] if version.startswith("16") else version[0]
        flavor = flavor_map.get(flavor_digit)
        if flavor:
            metadata_list.append(
                f"""                {{
                    "flavor": "{flavor}",
                    "interface": {version}
                }}"""
            )

    metadata_str = ",\n".join(metadata_list)

    with open(release_folder_path + "/release.json", "w") as rf:
        rf.write(
            f"""{{
    "releases": [
        {{
            "filename": "{zip_name}.zip",
            "nolib": false,
            "metadata": [
{metadata_str}
            ]
        }}
    ]
}}"""
        )

    print(f"New release '{release_dir}' created successfully")


def get_version_dir(is_release_build, versionOverride):
    version, nr_of_commits, recent_commit = get_git_information()
    if versionOverride != "":
        version = versionOverride
    print("Tag: " + version)
    if is_release_build:
        release_dir = "%s" % version
    else:
        release_dir = "%s-%s" % (version, recent_commit)

    print("Number of commits since tag: " + nr_of_commits)
    print("Most Recent commit: " + recent_commit)
    branch = get_branch()
    if branch != "master" and branch != "HEAD":
        release_dir += "-%s" % branch
    print("Current branch: " + branch)

    return release_dir


directoriesToInclude = ["Libs", "Modules", "Widgets"]
filesToInclude = [
    "icon.png",
    "QuestieTrace.lua",
    "QuestieTrace_UI.lua",
    "QuestieTrace-Classic.toc",
    "QuestieTrace-Camelot.toc",
]
ignorePatterns = ["*.test.lua"]


def copy_content_to(release_folder_path):
    for _, directories, files in os.walk("."):
        for directory in directories:
            if directory in directoriesToInclude:
                shutil.copytree(
                    directory,
                    "%s/%s" % (release_folder_path, directory),
                    ignore=shutil.ignore_patterns(*ignorePatterns),
                )
        for file in files:
            if file in filesToInclude:
                shutil.copy2(file, "%s/%s" % (release_folder_path, file))
        break


def zip_release_folder(zip_name, version_dir, addon_dir):
    root = os.getcwd()
    os.chdir("releases/%s" % version_dir)
    print("Zipping %s" % zip_name)
    shutil.make_archive(zip_name, "zip", ".", addon_dir)
    os.chdir(root)


def get_git_information():
    if is_tool("git"):
        script_dir = os.path.dirname(os.path.realpath(__file__))
        p = subprocess.check_output(["git", "describe", "--tags", "--long"], cwd=script_dir, stderr=subprocess.STDOUT)
        tag_string = str(p).rstrip("\\n'").lstrip("b'")

        # versiontag (v1.0.0) from git, number of additional commits on top of the tagged object and most recent commit.
        version_tag, nr_of_commits, recent_commit = tag_string.rsplit("-", maxsplit=2)
        recent_commit = recent_commit.lstrip("g")  # There is a "g" before all the commits.
        return version_tag, nr_of_commits, recent_commit
    else:
        raise RuntimeError("Warning: Git not found on the computer, unable to determine version.")


def get_branch():
    if is_tool("git"):
        script_dir = os.path.dirname(os.path.realpath(__file__))
        p = subprocess.check_output(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=script_dir)
        branch = str(p).rstrip("\\n'").lstrip("b'")
        return branch


def get_interface_versions():
    all_versions = []
    toc_files = ["QuestieTrace-Classic.toc", "QuestieTrace-Camelot.toc"]
    
    for toc_file in toc_files:
        with open(toc_file, "r") as toc:
            match = re.match("## Interface: (.*?)\n", toc.read(), re.DOTALL)
            if match:
                versions = [v.strip() for v in match.group(1).split(",")]
                all_versions.extend(versions)
    
    return all_versions


def is_tool(name):
    """Check whether `name` is on PATH and marked as executable."""
    return shutil.which(name) is not None


if __name__ == "__main__":
    main()
