/**
 * Compiler-API helpers shared by the `*-verify.ts` suite.
 *
 * Every function here answers a question about a parsed source file that a
 * regex over its text cannot: what a module imports and whether it calls a
 * function. The scripts that need them build their own `ts.SourceFile` and pass
 * it in — nothing here reads a file or builds a `ts.Program`, so nothing here
 * can resolve a type. Where a helper guesses lexically instead, its own doc
 * comment says so.
 */
import ts from "typescript";

/**
 * `src` parsed, with the script kind taken from `rel`'s extension.
 *
 * The kind is not cosmetic and defaulting it to TSX is a live bug: under TSX a
 * `.ts` file's `<T>(x: T) => x` is read as a JSX element, the parse recovers
 * somewhere unhelpful, and a walk over the result answers about a tree the file
 * does not have. Measured - a call pin over a `.ts` store went red under TSX
 * and green under TS, over identical source.
 */
export function parseSource(rel: string, src: string): ts.SourceFile {
  return ts.createSourceFile(
    rel,
    src,
    ts.ScriptTarget.ESNext,
    true,
    rel.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
}

/**
 * The names imported from `specifier` in `src`, and whether the clause brings
 * in types only. `null` when the module is not imported at all.
 *
 * The shape a POSITIVE check over raw source cannot have: a regex like
 * `/from "@\/lib\/searchTiers"/.test(src)` is satisfied by a sentence in the
 * file's own docblock explaining why it used to import that module, so the
 * check passes over the deletion it exists to catch. An import declaration read
 * AS a declaration cannot be spelled in a comment. (The NEGATIVE half of such a
 * pair is safe over raw text - prose reddens it, which costs a round rather
 * than a defect - so only the positives need this.)
 *
 * Both spellings of type-only count: `import type { A }` on the clause and
 * `import { type A }` per element say the same thing about whether a value
 * crosses the boundary, and a check that knows only one is about syntax.
 */
export function namedImportsFrom(
  rel: string,
  src: string,
  specifier: string,
): { names: string[]; typeOnly: boolean } | null {
  const sf = parseSource(rel, src);
  for (const st of sf.statements) {
    if (!ts.isImportDeclaration(st) || !ts.isStringLiteral(st.moduleSpecifier)) continue;
    if (st.moduleSpecifier.text !== specifier) continue;
    const bindings = st.importClause?.namedBindings;
    const named = bindings !== undefined && ts.isNamedImports(bindings) ? bindings.elements : [];
    return {
      names: named.map((e) => e.name.text).sort(),
      typeOnly:
        st.importClause?.isTypeOnly === true ||
        (named.length > 0 && named.every((e) => e.isTypeOnly)),
    };
  }
  return null;
}

/**
 * Does `src` CALL `name` - as a call expression, not as text?
 *
 * `src.includes("hostsUsingIdentity(")` is true of a comment that names the
 * call, including the comment somebody leaves behind when they inline it. A
 * method call of the same name counts, because the claim these callers make is
 * that the work is delegated rather than re-derived, and `x.foo()` delegates
 * exactly as much as `foo()`.
 */
export function callsFunction(rel: string, src: string, name: string): boolean {
  const sf = parseSource(rel, src);
  let found = false;
  const visit = (n: ts.Node): void => {
    if (found) return;
    if (ts.isCallExpression(n)) {
      const callee = n.expression;
      if (ts.isIdentifier(callee) && callee.text === name) found = true;
      else if (ts.isPropertyAccessExpression(callee) && callee.name.text === name) found = true;
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return found;
}
