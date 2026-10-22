import { expect, test } from "bun:test";
import fixtures from "../metadata/fixtures.json";
import schema from "../metadata/schema.json";
import { MAX_METADATA_BYTES, MAX_METADATA_DEPTH, parseMetadata, type MetadataKind } from "./metadata";

type ConformanceCase = { name: string; kind: string; valid: boolean; document?: unknown; raw?: string };
for (const { name, kind, valid, document, raw } of fixtures as ConformanceCase[]) {
  test(`metadata conformance: ${name}`, () => {
    const read = () => parseMetadata(kind as MetadataKind, Buffer.from(raw ?? JSON.stringify(document)));
    if (valid) expect(read()).toEqual(document as Record<string, unknown>);
    else expect(read).toThrow();
  });
}

test("schema examples validate with explicit kind", () => {
  for (const kind of ["provider", "model"] as const) {
    for (const document of schema.$defs[kind].examples) expect(parseMetadata(kind, Buffer.from(JSON.stringify(document)))).toEqual(document);
  }
});

test("metadata rejects oversized bytes, malformed UTF-8 and invalid JSON", () => {
  for (const bytes of [Buffer.alloc(MAX_METADATA_BYTES + 1, 32), Buffer.from([0xff]), Buffer.from("{broken")]) expect(() => parseMetadata("provider", bytes)).toThrow();
});

test("integer JSON notation stays compatible across validators", () => {
  const bytes = Buffer.from('{"id":"example/model","name":"Model","modalities":{"input":["text"],"output":["text"]},"contextTokens":100.0,"outputTokens":10e0}');
  expect(parseMetadata("model", bytes).contextTokens).toBe(100);
});

test("parser rejects BOM, unpaired surrogates, non-finite numbers and excessive extension nesting", () => {
  const provider = '{"name":"Example","endpoint":"https://example.com/v1"}';
  for (const text of [`\ufeff${provider}`, provider.replace("Example", "\\ud800"), provider.replace("Example", "\\udc00"), provider.replace("Example", "\\udc00\\ud800"), provider.replace("}", ',"extension":1e999}'), provider.replace('"name"', '"\\ud800"')]) expect(() => parseMetadata("provider", Buffer.from(text))).toThrow();
  let extension: unknown = {};
  for (let index = 0; index < MAX_METADATA_DEPTH - 2; index++) extension = { nested: extension };
  const value = { name: "Example", endpoint: "https://example.com/v1", extension };
  expect(parseMetadata("provider", Buffer.from(JSON.stringify(value)))).toEqual(value);
  value.extension = { nested: extension };
  expect(() => parseMetadata("provider", Buffer.from(JSON.stringify(value)))).toThrow("container levels");
});
