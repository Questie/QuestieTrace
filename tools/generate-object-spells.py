#!/usr/bin/env python3
"""Generate Modules/ObjectSpells.lua: the spells the player casts on world objects.

Usage (dbc-source.db is the DBC release database):
    uv run tools/generate-object-spells.py <path to dbc-source.db>
    uv run tools/generate-object-spells.py --check <path to dbc-source.db>

--check compares the spell IDs with those in Modules/ObjectSpells.lua instead of writing it, and exits
with 3 when they differ. Names and the build in the header are not compared: they change by themselves.

A spell is included when one of its effects targets a game object, or a game object or item.
That is what makes the target of a cast safe to record: a spell that could also target a unit
could carry a player's name, so every other target type of an included spell must be one of
NON_UNIT_TARGETS. An unlisted one stops the generator instead of being guessed at; check what it
selects in WoWDBDefs (meta/enums/Target.dbde) before adding it.
"""

import argparse
import re
import sqlite3
import sys
from pathlib import Path

OUTPUT = Path(__file__).resolve().parent.parent / "Modules" / "ObjectSpells.lua"

# Implicit targets the player picks: the object, or the object or item.
OBJECT_TARGETS = {23: "TARGET_GAMEOBJECT_TARGET", 26: "TARGET_GAMEOBJECT_ITEM_TARGET"}
# Targets that can accompany them. None of these selects another unit.
NON_UNIT_TARGETS = {
    0: "none",
    1: "TARGET_UNIT_CASTER (the player)",
    18: "TARGET_DEST_CASTER",
    40: "TARGET_GAMEOBJECT_NEARBY_ENTRY",
    47: "TARGET_DEST_CASTER_FRONT",
    87: "TARGET_DEST_DEST",
}


def read_spells(connection: sqlite3.Connection) -> dict[int, tuple[str, set[int]]]:
    """Every spell with an object target: its name and all of its implicit targets."""
    spells: dict[int, tuple[str, set[int]]] = {}
    rows = connection.execute(
        """
        SELECT effect.SpellID, name.Name_lang, effect.ImplicitTarget_0, effect.ImplicitTarget_1
        FROM spell_effect_v1 AS effect
        LEFT JOIN spell_name_v1 AS name ON name.ID = effect.SpellID
        WHERE effect.SpellID IN (
            SELECT SpellID FROM spell_effect_v1
            WHERE ImplicitTarget_0 IN (23, 26) OR ImplicitTarget_1 IN (23, 26)
        )
        ORDER BY effect.SpellID
        """
    )
    for spell_id, name, *targets in rows:
        if None in targets:
            sys.exit(f"Spell {spell_id} has an effect without a target")
        spells.setdefault(int(spell_id), (name or "?", set()))[1].update(int(target) for target in targets)
    return spells


def check_targets(spells: dict[int, tuple[str, set[int]]]) -> None:
    allowed = OBJECT_TARGETS.keys() | NON_UNIT_TARGETS.keys()
    rejected = [(spell_id, name, sorted(targets - allowed)) for spell_id, (name, targets) in spells.items() if targets - allowed]
    if rejected:
        lines = "\n".join(f"  {spell_id} {name}: {targets}" for spell_id, name, targets in rejected)
        sys.exit(f"Spells with a target type that is not in NON_UNIT_TARGETS:\n{lines}")


def client_build(connection: sqlite3.Connection) -> str:
    """The newest build of major version 1 in the database, which its _v1 views export. Major 1 also
    covers Classic Era (1.15), whose spells differ, so it has to be a Forever (1.60) build."""
    (build,) = connection.execute(
        "SELECT build FROM _build WHERE major_version = 1 ORDER BY build_version DESC LIMIT 1"
    ).fetchone()
    if not build.startswith("1.60."):
        sys.exit(f"The newest build of major version 1 is {build}, not a Forever (1.60) build")
    return build


def render(spells: dict[int, tuple[str, set[int]]], build: str) -> str:
    entries = "\n".join(f"  [{spell_id}] = true, -- {name}" for spell_id, (name, _) in spells.items())
    return f"""---@class QuestieTraceCore
local Core = QuestieTraceCore

-- AUTO GENERATED FILE! DO NOT EDIT! Regenerate with tools/generate-object-spells.py.
-- Client build {build}: the {len(spells)} spells that target a game object (or an object or item) and no other unit.

---@type table<number, boolean>
Core.OBJECT_SPELLS = {{
{entries}
}}
"""


def compare_with_list(spells: dict[int, tuple[str, set[int]]], build: str) -> int:
    """Print what the list lacks or has too much of; returns the process's exit code."""
    listed = {int(spell_id) for spell_id in re.findall(r"^  \[(\d+)\] = true,", OUTPUT.read_text(encoding="utf-8"), re.MULTILINE)}
    if not listed:
        sys.exit(f"{OUTPUT} has no spells in it")
    missing = sorted(spells.keys() - listed)
    extra = sorted(listed - spells.keys())
    if not missing and not extra:
        print(f"{OUTPUT.name} has the {len(listed)} spells of build {build}")
        return 0
    print(f"{OUTPUT.name} differs from build {build}:")
    for spell_id in missing:
        print(f"  missing {spell_id} {spells[spell_id][0]}")
    for spell_id in extra:
        print(f"  no longer targets an object: {spell_id}")
    return 3


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Modules/ObjectSpells.lua")
    parser.add_argument("database", type=Path, help="the DBC release database, dbc-source.db")
    parser.add_argument("--check", action="store_true", help="compare the spell IDs instead of writing the list")
    arguments = parser.parse_args()
    database = arguments.database.resolve()
    if not database.is_file():
        sys.exit(f"{database} does not exist")
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    spells = read_spells(connection)
    if not spells:
        sys.exit("The database has no spells with an object target")
    check_targets(spells)
    build = client_build(connection)
    if arguments.check:
        sys.exit(compare_with_list(spells, build))
    OUTPUT.write_text(render(spells, build), encoding="utf-8", newline="\n")
    print(f"Wrote {len(spells)} spells of build {build} to {OUTPUT}")


if __name__ == "__main__":
    main()
