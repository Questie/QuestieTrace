// npcKeys.name - Questie field 1.

import { unitNameAt } from "../../probes";
import type { FieldObserver, Observation } from "../../observation";
import { npcEncounters } from "./_encounters";

export const observeName: FieldObserver<string> = (session) => {
  const observations: Observation<string>[] = [];
  for (const encounter of npcEncounters(session)) {
    const name = unitNameAt(session, encounter.token, encounter.t);
    if (name) {
      observations.push({
        entityId: encounter.npcID,
        value: name,
        confidence: "high",
        provenance: { session: session.name, t: encounter.t },
      });
    }
  }
  return observations;
};
