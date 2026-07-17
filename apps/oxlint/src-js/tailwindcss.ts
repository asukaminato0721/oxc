import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

type Request = {
  operation:
    | "canonicalClasses"
    | "classOrder"
    | "conflictingClasses"
    | "unknownClasses"
    | "variantOrder";
  filePath: string;
  settings: Record<string, unknown>;
  classes: string[];
  options: Record<string, unknown>;
};

type DesignSystem = {
  candidatesToCss(classes: string[]): Array<string | null>;
  canonicalizeCandidates?(classes: string[], options: unknown): string[];
  compileAstNodes(candidate: unknown): Array<{ node?: { nodes?: AstNode[] } }>;
  getClassOrder(classes: string[]): Array<[string, bigint | null]>;
  getVariantOrder(): Map<unknown, number>;
  getVariants(): Array<{ name: string; selectors?(): string[] }>;
  parseCandidate(className: string): unknown[];
  printVariant(variant: unknown): string;
};

type AstNode = {
  kind: string;
  important?: boolean;
  name?: string;
  nodes?: AstNode[];
  params?: string;
  property?: string;
  selector?: string;
  value?: string;
};

const contexts = new Map<string, Promise<DesignSystem>>();

export async function queryTailwindDesignSystem(serializedRequest: string): Promise<string> {
  const request = JSON.parse(serializedRequest) as Request;
  const design = await getDesignSystem(request);
  let result: unknown;
  switch (request.operation) {
    case "classOrder":
      result = design
        .getClassOrder(request.classes)
        .map(([className, order]) => [className, order?.toString() ?? null]);
      break;
    case "unknownClasses":
      {
        const css = design.candidatesToCss(request.classes);
        result = request.classes.filter((_, index) => css[index] === null);
      }
      break;
    case "canonicalClasses":
      result = canonicalClasses(design, request.classes, request.options);
      break;
    case "variantOrder":
      result = variantOrder(design, request.classes);
      break;
    case "conflictingClasses":
      result = conflictingClasses(design, request.classes);
      break;
  }
  return JSON.stringify(result);
}

async function getDesignSystem(request: Request): Promise<DesignSystem> {
  const settings = (request.settings["better-tailwindcss"] as Record<string, unknown>) ?? {};
  const cwdValue = typeof settings.cwd === "string" ? settings.cwd : undefined;
  const cwd = resolve(cwdValue ?? process.cwd());
  const entryValue = typeof settings.entryPoint === "string" ? settings.entryPoint : undefined;
  const entryPoint = entryValue
    ? isAbsolute(entryValue)
      ? entryValue
      : resolve(cwd, entryValue)
    : undefined;
  const key = `${cwd}\0${entryPoint ?? "<default>"}`;
  let context = contexts.get(key);
  if (!context) {
    context = loadDesignSystem(cwd, entryPoint);
    contexts.set(key, context);
  }
  return context;
}

async function loadDesignSystem(
  cwd: string,
  entryPoint: string | undefined,
): Promise<DesignSystem> {
  const projectRequire = createRequire(join(cwd, "package.json"));
  const tailwindPath = projectRequire.resolve("tailwindcss");
  const { __unstable__loadDesignSystem } = (await import(pathToFileURL(tailwindPath).href)) as {
    __unstable__loadDesignSystem(
      css: string,
      options: Record<string, unknown>,
    ): Promise<DesignSystem>;
  };
  const css = entryPoint ? await readFile(entryPoint, "utf8") : '@import "tailwindcss";';
  const base = entryPoint ? dirname(entryPoint) : cwd;

  return __unstable__loadDesignSystem(css, {
    base,
    loadModule: async (id: string, moduleBase: string) => {
      const moduleRequire = createRequire(join(moduleBase, "package.json"));
      const resolved = moduleRequire.resolve(id);
      const imported = await import(pathToFileURL(resolved).href);
      return { base: dirname(resolved), module: imported.default ?? imported };
    },
    loadStylesheet: async (id: string, stylesheetBase: string) => {
      const stylesheetRequire = createRequire(join(stylesheetBase, "package.json"));
      const request = id === "tailwindcss" ? "tailwindcss/index.css" : id;
      const resolved = stylesheetRequire.resolve(request);
      return { base: dirname(resolved), content: await readFile(resolved, "utf8") };
    },
  });
}

function canonicalClasses(
  design: DesignSystem,
  classes: string[],
  options: Record<string, unknown>,
): Record<string, { input: string[]; output: string }> {
  const result: Record<string, { input: string[]; output: string }> = {};
  if (!design.canonicalizeCandidates) {
    for (const className of classes) {
      result[className] = { input: [className], output: className };
    }
    return result;
  }
  const css = design.candidatesToCss(classes);
  const unknown = classes.filter((_, index) => css[index] === null);
  const known = classes.filter((className) => !unknown.includes(className));
  const canonicalized = design.canonicalizeCandidates(known, options);
  const removed = known.filter((className) => !canonicalized.includes(className));
  for (const className of [...known, ...unknown]) {
    if (canonicalized.includes(className) || unknown.includes(className)) {
      result[className] = { input: [className], output: className };
    }
  }
  for (const canonical of canonicalized.filter((className) => !classes.includes(className))) {
    const necessary = removed.filter((removedClass) => {
      const subset = removed.filter((className) => className !== removedClass);
      return !design.canonicalizeCandidates!(subset, options).includes(canonical);
    });
    for (const original of necessary) {
      result[original] = { input: necessary, output: canonical };
    }
  }
  return result;
}

function variantOrder(design: DesignSystem, classes: string[]): Record<string, number> {
  const variantsByName = new Map(
    (design.getVariants() ?? []).map((variant) => [variant.name, variant]),
  );
  const orderByName = new Map(
    [...design.getVariantOrder()].map(([variant, order]) => [design.printVariant(variant), order]),
  );
  const result: Record<string, number> = {};
  for (const className of classes) {
    for (const candidate of design.parseCandidate(className) as Array<{
      variants?: unknown[];
    }>) {
      for (const variantCandidate of candidate.variants ?? []) {
        const name = design.printVariant(variantCandidate);
        const variant = variantsByName.get(name);
        const selectors = variant?.selectors?.();
        const global =
          Array.isArray(selectors) &&
          selectors.length > 0 &&
          selectors.every((selector) => typeof selector === "string" && !selector.includes("&"))
            ? 1 << 30
            : 0;
        result[name] ??= global | (orderByName.get(name) ?? 0);
      }
    }
  }
  return result;
}

type Property = { cssPropertyName: string; important: boolean };
type RuleContext = Record<string, Property[]>;

function conflictingClasses(
  design: DesignSystem,
  classes: string[],
): Record<string, Record<string, Property[]>> {
  const rules: Record<string, RuleContext> = {};
  for (const className of classes) {
    const context: RuleContext = {};
    for (const candidate of design.parseCandidate(className)) {
      const [rule] = design.compileAstNodes(candidate);
      collectRuleContext(rule?.node?.nodes, context);
    }
    rules[className] = context;
  }
  const conflicts: Record<string, Record<string, Property[]>> = {};
  for (const className of classes) {
    for (const otherClassName of classes) {
      if (className === otherClassName) continue;
      const left = rules[className];
      const right = rules[otherClassName];
      const paths = Object.keys(left);
      if (
        paths.length === 0 ||
        paths.length !== Object.keys(right).length ||
        paths.some(
          (path) =>
            !right[path] ||
            left[path].length !== right[path].length ||
            left[path].some(
              (property) =>
                !right[path].some((other) => other.cssPropertyName === property.cssPropertyName),
            ),
        )
      ) {
        continue;
      }
      conflicts[className] ??= {};
      conflicts[className][otherClassName] = paths.flatMap((path) => right[path]);
    }
  }
  return conflicts;
}

function collectRuleContext(nodes: AstNode[] | undefined, context: RuleContext, path = ""): void {
  for (const node of nodes ?? []) {
    if (node.kind === "declaration") {
      if (node.value !== undefined && node.property) {
        (context[path] ??= []).push({
          cssPropertyName: node.property,
          important: node.important ?? false,
        });
      }
    } else if (node.kind === "rule") {
      collectRuleContext(node.nodes, context, path + (node.selector ?? ""));
    } else if (node.kind === "at-rule") {
      collectRuleContext(node.nodes, context, path + (node.name ?? "") + (node.params ?? ""));
    }
  }
}
