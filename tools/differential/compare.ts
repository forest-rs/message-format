// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

type JsonValue = null | boolean | number | string | JsonValue[] | {
  [key: string]: JsonValue;
};

type Case = {
  id: string;
  src: string;
  locale: string;
  bidiIsolation: "default" | "none";
  params: Record<string, JsonValue>;
};

type Observation = {
  id: string;
  phase: "compile" | "input" | "format" | "internal";
  output?: string;
  errors: string[];
  parts: JsonValue[];
};

const DATA_MODEL_ERRORS = new Map([
  ["duplicate-declaration", "duplicate-declaration"],
  ["duplicate-option-name", "duplicate-option-name"],
  ["duplicate-variant", "duplicate-variant"],
  ["key-mismatch", "variant-key-mismatch"],
  ["missing-fallback", "missing-fallback-variant"],
  ["missing-selector-annotation", "missing-selector-annotation"],
]);

const MUTATION_VALUES: JsonValue[] = [
  null,
  false,
  true,
  -2,
  -1,
  0,
  1,
  1.5,
  2,
  11,
  1000,
  1000000.5,
  "",
  "0",
  "1",
  "1.50",
  "foo",
  "é",
  "é",
  "مرحبا",
];

const detailed = Deno.args.includes("--all");
const paths = Deno.args.filter((arg) => !arg.startsWith("--"));
const repoRoot = await Deno.realPath(new URL("../..", import.meta.url));
const jsRoot = await Deno.realPath(
  paths[0] ?? `${repoRoot}/../messageformat`,
);
const wgRoot = await Deno.realPath(
  paths[1] ?? `${repoRoot}/../message-format-wg/test/tests`,
);
const toFileUrl = (path: string) => new URL(`file://${path}`).href;
const messageformat = await import(
  toFileUrl(`${jsRoot}/mf2/messageformat/src/index.ts`)
);
const { DraftFunctions } = await import(
  toFileUrl(`${jsRoot}/mf2/messageformat/src/functions/index.ts`)
);
const { TestFunctions } = await import(
  toFileUrl(`${jsRoot}/mf2/messageformat/src/functions/test-functions.ts`)
);
const functions = { ...DraftFunctions, ...TestFunctions };

function normalizeError(error: unknown): string {
  if (typeof error !== "object" || error === null || !("type" in error)) {
    return String(error);
  }
  const type = String(error.type);
  return DATA_MODEL_ERRORS.get(type) ?? type;
}

function normalizeCompileError(error: unknown): string {
  const normalized = normalizeError(error);
  return DATA_MODEL_ERRORS.has(normalized) ||
      [...DATA_MODEL_ERRORS.values()].includes(normalized)
    ? normalized
    : "syntax-error";
}

function jsParams(params: Record<string, JsonValue>): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(params).map(([name, value]) => {
      if (
        typeof value === "object" &&
        value !== null &&
        !Array.isArray(value) &&
        typeof value.$datetime === "string"
      ) {
        return [name, new Date(value.$datetime)];
      }
      return [name, value];
    }),
  );
}

function observeJs(tc: Case): Observation {
  let mf;
  try {
    mf = new messageformat.MessageFormat(tc.locale, tc.src, {
      bidiIsolation: tc.bidiIsolation,
      functions,
    });
  } catch (error) {
    return {
      id: tc.id,
      phase: "compile",
      errors: [normalizeCompileError(error)],
      parts: [],
    };
  }
  const params = jsParams(tc.params);
  const errors: unknown[] = [];
  const output = mf.format(params, (error: unknown) => errors.push(error));
  const partErrors: unknown[] = [];
  const parts = mf.formatToParts(
    params,
    (error: unknown) => partErrors.push(error),
  );
  const normalized = errors.map(normalizeError);
  const normalizedParts = partErrors.map(normalizeError);
  if (JSON.stringify(normalized) !== JSON.stringify(normalizedParts)) {
    normalized.push(`parts-errors:${normalizedParts.join(",")}`);
  }
  return {
    id: tc.id,
    phase: "format",
    output,
    errors: normalized,
    parts,
  };
}

async function jsonFiles(root: string): Promise<string[]> {
  const files: string[] = [];
  for await (const entry of Deno.readDir(root)) {
    const path = `${root}/${entry.name}`;
    if (entry.isDirectory) files.push(...await jsonFiles(path));
    else if (entry.isFile && entry.name.endsWith(".json")) files.push(path);
  }
  return files.sort();
}

function paramsFromWg(params: unknown): Record<string, JsonValue> {
  if (!Array.isArray(params)) return {};
  return Object.fromEntries(params.map((param) => [
    param.name,
    param.type === "datetime" ? { $datetime: param.value } : param.value,
  ]));
}

function hasCompileError(expErrors: unknown): boolean {
  if (!Array.isArray(expErrors)) return false;
  return expErrors.some((error) =>
    error?.type === "syntax-error" ||
    DATA_MODEL_ERRORS.has(error?.type) ||
    [...DATA_MODEL_ERRORS.values()].includes(error?.type)
  );
}

async function corpus(): Promise<Case[]> {
  const base: Array<Case & { compileError: boolean }> = [];
  for (const file of await jsonFiles(wgRoot)) {
    const suite = JSON.parse(await Deno.readTextFile(file));
    const relative = file.slice(wgRoot.length + 1);
    for (const [index, test] of suite.tests.entries()) {
      const tc = { ...suite.defaultTestProperties, ...test };
      base.push({
        id: `wg/${relative}:${index + 1}`,
        src: tc.src,
        locale: tc.locale ?? "en-US",
        bidiIsolation: tc.bidiIsolation ?? "none",
        params: paramsFromWg(tc.params),
        compileError: hasCompileError(tc.expErrors),
      });
    }
  }

  const cases: Case[] = base.map(({ compileError: _, ...tc }) => tc);
  for (const tc of base) {
    if (tc.compileError) continue;
    for (const name of Object.keys(tc.params)) {
      if (
        typeof tc.params[name] === "object" &&
        tc.params[name] !== null
      ) continue;
      for (const [index, value] of MUTATION_VALUES.entries()) {
        cases.push({
          id: `${tc.id}/mutate/${name}/${index}`,
          src: tc.src,
          locale: tc.locale,
          bidiIsolation: tc.bidiIsolation,
          params: { ...tc.params, [name]: value },
        });
      }
    }
  }
  return cases;
}

async function observeRust(cases: Case[]): Promise<Observation[]> {
  const command = new Deno.Command("cargo", {
    args: [
      "run",
      "--quiet",
      "-p",
      "message-format-conformance",
      "--bin",
      "mf2_observer",
    ],
    cwd: repoRoot,
    stdin: "piped",
    stdout: "piped",
    stderr: "inherit",
  });
  const child = command.spawn();
  const outputPromise = child.output();
  const writer = child.stdin.getWriter();
  const encoded = new TextEncoder().encode(
    cases.map((tc) => JSON.stringify(tc)).join("\n") + "\n",
  );
  await writer.write(encoded);
  await writer.close();
  const result = await outputPromise;
  if (!result.success) throw new Error(`Rust observer exited ${result.code}`);
  return new TextDecoder()
    .decode(result.stdout)
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function sorted(values: string[]): string[] {
  return [...values].sort();
}

function canonical(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map(canonical);
  if (typeof value !== "object" || value === null) return value;
  return Object.fromEntries(
    Object.entries(value)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, item]) => [key, canonical(item)]),
  );
}

function countsByFile(
  items: Array<{ case: Case }>,
): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const { case: tc } of items) {
    const file = tc.id.split(":")[0];
    counts[file] = (counts[file] ?? 0) + 1;
  }
  return counts;
}

function representativesByFile(
  items: Array<{ case: Case; rust: Observation; js: Observation }>,
): unknown[] {
  const seen = new Set<string>();
  const representatives = [];
  for (const { case: tc, rust: rs, js } of items) {
    const file = tc.id.split(":")[0];
    if (seen.has(file)) continue;
    seen.add(file);
    representatives.push({
      case: tc,
      rust: { phase: rs.phase, output: rs.output, errors: rs.errors },
      js: { phase: js.phase, output: js.output, errors: js.errors },
    });
  }
  return representatives;
}

const cases = await corpus();
const rust = await observeRust(cases);
const disagreements: Array<{
  case: Case;
  rust: Observation;
  js: Observation;
}> = [];
const partDisagreements: Array<{
  id: string;
  rust: JsonValue[];
  js: JsonValue[];
}> = [];
for (const [index, tc] of cases.entries()) {
  const js = observeJs(tc);
  const rs = rust[index];
  const semanticMatch = js.id === rs.id &&
    js.phase === rs.phase &&
    js.output === rs.output &&
    JSON.stringify(sorted(js.errors)) === JSON.stringify(sorted(rs.errors));
  if (!semanticMatch) {
    disagreements.push({ case: tc, rust: rs, js });
  } else if (
    js.phase === "format" &&
    JSON.stringify(canonical(js.parts)) !== JSON.stringify(canonical(rs.parts))
  ) {
    partDisagreements.push({ id: tc.id, rust: rs.parts, js: js.parts });
  }
}

console.log(JSON.stringify(
  {
    cases: cases.length,
    semanticDisagreements: disagreements.length,
    baseSemanticDisagreements: disagreements.filter(({ case: tc }) =>
      !tc.id.includes("/mutate/")
    ).length,
    mutatedSemanticDisagreements: disagreements.filter(({ case: tc }) =>
      tc.id.includes("/mutate/")
    ).length,
    semanticDisagreementsByFile: countsByFile(disagreements),
    structuredPartDisagreements: partDisagreements.length,
    baseDisagreements: disagreements
      .filter(({ case: tc }) => !tc.id.includes("/mutate/"))
      .map(({ case: tc, rust: rs, js }) => ({
        id: tc.id,
        src: tc.src,
        rust: {
          phase: rs.phase,
          output: rs.output,
          errors: rs.errors,
        },
        js: { phase: js.phase, output: js.output, errors: js.errors },
      })),
    mutatedRepresentatives: representativesByFile(
      disagreements.filter(({ case: tc }) => tc.id.includes("/mutate/")),
    ),
    ...(detailed ? { disagreements } : {}),
    partDisagreementSample: partDisagreements.slice(0, 3),
  },
  null,
  2,
));

if (disagreements.length > 0) Deno.exit(1);
