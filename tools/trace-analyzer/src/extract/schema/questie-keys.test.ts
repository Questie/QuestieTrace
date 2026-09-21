import { describe, expect, it } from "vitest";
import { itemKeys, npcKeys, objectKeys, questKeys, type QuestieEntitySchema } from "./questie-keys";

const entities: Record<string, QuestieEntitySchema> = {
  npcKeys,
  questKeys,
  itemKeys,
  objectKeys,
};

describe("questie-keys schema", () => {
  for (const [entityName, schema] of Object.entries(entities)) {
    it(`should have contiguous 1-based indices for ${entityName}`, () => {
      const indices = Object.values(schema)
        .map((f) => f.index)
        .sort((a, b) => a - b);
      expect(indices).toEqual(Array.from({ length: indices.length }, (_, i) => i + 1));
    });

    it(`should have string/table fields default to "" or null (never undefined) for ${entityName}`, () => {
      for (const [fieldName, field] of Object.entries(schema)) {
        expect(field.default, `${entityName}.${fieldName} default`).not.toBeUndefined();
        if (field.type === "string") {
          expect(
            field.default === "" || field.default === null,
            `${entityName}.${fieldName} is type "string" but default is ${JSON.stringify(field.default)}`,
          ).toBe(true);
        }
        if (field.type === "table") {
          expect(
            field.default === null,
            `${entityName}.${fieldName} is type "table" but default is ${JSON.stringify(field.default)}`,
          ).toBe(true);
        }
      }
    });
  }
});
