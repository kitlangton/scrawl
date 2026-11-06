import ts from 'typescript';
import { execFileSync } from 'node:child_process';
import { readFileSync, existsSync, realpathSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export type Owner = 'engine' | 'record' | 'agent' | 'onboarding' | 'connect' | 'shared' | 'composition' | 'legacy' | 'other';
const product = ['engine', 'record', 'agent', 'onboarding'] as const;
const packageRoots: Record<string, string> = {
  '@apparent/engine-sdk': 'engine/sdk', '@apparent/record': 'record',
  '@apparent/agent': 'agent', '@apparent/onboarding': 'onboarding',
  '@apparent/native-host': 'composition/native-host',
  '@apparent/connect-client': 'connect/delivery/client',
  '@apparent/connect-static-hosting': 'connect/tooling/hosting',
  '@apparent/app-contracts': 'shared/contracts', '@apparent/ui': 'shared/ui',
};
const packageOwners: Record<string, Owner> = {
  '@apparent/engine-sdk': 'engine', '@apparent/record': 'record', '@apparent/agent': 'agent',
  '@apparent/onboarding': 'onboarding', '@apparent/app-contracts': 'shared', '@apparent/ui': 'shared',
  '@apparent/connect-client': 'connect', '@apparent/connect-static-hosting': 'connect',
  '@apparent/native-host': 'composition',
};
const publicEdges: Record<Owner, Owner[]> = {
  engine: ['shared'], record: ['engine', 'shared'], agent: ['engine', 'record', 'shared'], onboarding: ['record', 'shared'],
  connect: ['shared'], shared: [], composition: ['engine', 'record', 'agent', 'onboarding', 'shared'], legacy: [], other: [],
};
const sourceExt = new Set(['.ts', '.tsx', '.cts', '.mts']);
const ignoredParts = new Set(['node_modules', 'dist', 'target', '.git']);
const gitEnvironment = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('GIT_')));

function ownerOf(root: string, file: string): Owner {
  const rel = path.relative(root, file).split(path.sep);
  if (rel[0] === 'engine') return 'engine';
  if (rel[0] === 'record') return 'record';
  if (rel[0] === 'agent') return 'agent';
  if (rel[0] === 'onboarding') return 'onboarding';
  if (rel[0] === 'connect' && (rel[1] === 'service' || (rel[1] === 'delivery' && rel[2] === 'client') || (rel[1] === 'tooling' && rel[2] === 'hosting'))) return 'connect';
  if (rel[0] === 'shared') return 'shared';
  if (rel[0] === 'composition') return 'composition';
  if (rel[0] === 'capture') return 'legacy';
  return 'other';
}
function allowed(from: Owner, to: Owner): boolean { return from === to || publicEdges[from].includes(to); }

// This package exception does not authorize composition source paths or aliases.
function allowedPackage(root: string, source: string, from: Owner, to: Owner, name: string, subpath = '.'): boolean {
  const nativeHostRelative = path.relative(canonical(path.resolve(root, 'composition/native-host')), canonical(source));
  const inNativeHost = nativeHostRelative !== '..' && !nativeHostRelative.startsWith(`..${path.sep}`) && !path.isAbsolute(nativeHostRelative);
  if (inNativeHost && to === 'agent') return false;
  return allowed(from, to) || (from === 'agent' && to === 'composition' && name === '@apparent/native-host'
    && (subpath === '.' || subpath === './clipboard'));
}

function manifestExports(root: string, pkg: string, packageNames: Map<string,{owner:Owner;dir:string}>): Set<string> {
  const dir = packageNames.get(pkg)?.dir ?? path.resolve(root, packageRoots[pkg] ?? '');
  try {
    const value = JSON.parse(readFileSync(path.join(dir, 'package.json'), 'utf8')) as { exports?: Record<string, unknown> | string };
    if (typeof value.exports === 'string') return new Set(['.']);
    return new Set(Object.keys(value.exports ?? {}));
  } catch { return new Set(); }
}
function packageRef(spec: string, packageNames: Map<string,{owner:Owner;dir:string}>): { name: string; subpath: string } | undefined {
  if (spec.startsWith('.') || spec.startsWith('/') || spec.startsWith('file:') || spec.startsWith('#')) return;
  const bits = spec.split('/');
  const name = spec.startsWith('@') ? bits.slice(0, 2).join('/') : bits[0];
  return packageOwners[name] || packageNames.has(name) ? { name, subpath: bits.length > (name.startsWith('@') ? 2 : 1) ? './' + bits.slice(name.startsWith('@') ? 2 : 1).join('/') : '.' } : undefined;
}
type Alias = { pattern: string; targets: string[] };
function configScope(root: string, source: string): { aliases: Alias[]; options?: ts.CompilerOptions } {
  let dir = path.dirname(source);
  while (path.relative(root, dir) !== '..' && !path.relative(root, dir).startsWith(`..${path.sep}`)) {
    const config = path.join(dir, 'tsconfig.json');
    if (existsSync(config)) {
      try {
        const parsed = ts.getParsedCommandLineOfConfigFile(config, {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic: () => {} });
        if (!parsed) return { aliases: [] };
        const inheritedBase = parsed.options.pathsBasePath;
        const base = parsed.options.baseUrl ?? (typeof inheritedBase === 'string' ? inheritedBase : dir);
        return { options: parsed.options, aliases: Object.entries(parsed.options.paths ?? {}).map(([pattern, targets]) => ({ pattern, targets: targets.map(t => path.resolve(base, t)) })) };
      } catch { return { aliases: [] }; }
    }
    const parent = path.dirname(dir); if (parent === dir) break; dir = parent;
  }
  return { aliases: [] };
}
function aliasTargets(spec: string, aliases: Alias[]): string[] {
  const out: string[] = [];
  for (const { pattern, targets } of aliases) {
    const [before, after] = pattern.split('*');
    if (after === undefined ? spec === pattern : spec.startsWith(before) && spec.endsWith(after)) {
      const middle = after === undefined ? '' : spec.slice(before.length, spec.length - after.length);
      out.push(...targets.map(t => t.replace('*', middle)));
    }
  }
  return out;
}
function canonical(p: string): string {
  let current = path.resolve(p);
  while (!existsSync(current)) { const parent = path.dirname(current); if (parent === current) return current; current = parent; }
  const real = realpathSync(current);
  return path.resolve(real, path.relative(current, path.resolve(p)));
}
function resolveLocal(root: string, source: string, spec: string): string {
  let candidate: string;
  try { candidate = spec.startsWith('file:') ? fileURLToPath(spec) : (path.isAbsolute(spec) ? spec : path.resolve(path.dirname(source), spec)); }
  catch { return path.resolve(root, '__invalid_file_url__'); }
  return canonical(candidate);
}

type FixtureImport = { function: string; env: string; package: string; exports: string[] };

/** Owner-declared test loaders may resolve only named public package entries. */
function publicFixtureImport(root: string, source: ts.SourceFile, node: ts.CallExpression, rule: FixtureImport,
  packageNames: Map<string,{owner:Owner;dir:string}>): string | undefined {
  const argument = node.arguments[0];
  if (!argument || !ts.isPropertyAccessExpression(argument) || argument.name.text !== 'href'
    || !ts.isCallExpression(argument.expression) || !ts.isIdentifier(argument.expression.expression)
    || argument.expression.expression.text !== 'pathToFileURL' || argument.expression.arguments.length !== 1
    || !ts.isIdentifier(argument.expression.arguments[0])) return;
  const importedUrlHelper = source.statements.some(statement => ts.isImportDeclaration(statement)
    && ts.isStringLiteral(statement.moduleSpecifier) && statement.moduleSpecifier.text === 'node:url'
    && statement.importClause?.namedBindings && ts.isNamedImports(statement.importClause.namedBindings)
    && statement.importClause.namedBindings.elements.some(element => !element.propertyName && element.name.text === 'pathToFileURL'));
  if (!importedUrlHelper) return;
  let parent: ts.Node | undefined = node.parent;
  while (parent && !ts.isFunctionDeclaration(parent)) {
    if (ts.isArrowFunction(parent) || ts.isFunctionExpression(parent)) return;
    parent = parent.parent;
  }
  if (!parent || !ts.isFunctionDeclaration(parent) || parent.name?.text !== rule.function || !parent.body) return;
  const constants = new Map<string, ts.Expression>();
  for (const statement of parent.body.statements) {
    if (!ts.isVariableStatement(statement) || !(statement.declarationList.flags & ts.NodeFlags.Const)) continue;
    for (const declaration of statement.declarationList.declarations) {
      if (ts.isIdentifier(declaration.name) && declaration.initializer) constants.set(declaration.name.text, declaration.initializer);
    }
  }
  const entry = constants.get(argument.expression.arguments[0].text);
  if (!entry || !ts.isTemplateExpression(entry) || entry.head.text || entry.templateSpans.length !== 1) return;
  const span = entry.templateSpans[0];
  if (!ts.isIdentifier(span.expression)) return;
  const consumer = constants.get(span.expression.text);
  if (!consumer || !ts.isPropertyAccessExpression(consumer) || consumer.name.text !== rule.env
    || !ts.isPropertyAccessExpression(consumer.expression) || consumer.expression.name.text !== 'env'
    || !ts.isIdentifier(consumer.expression.expression) || consumer.expression.expression.text !== 'process') return;
  try {
    const packageRoot = packageNames.get(rule.package)?.dir ?? (packageRoots[rule.package] && path.resolve(root, packageRoots[rule.package]));
    if (!packageRoot) return;
    const manifest = JSON.parse(readFileSync(path.join(packageRoot, 'package.json'), 'utf8'));
    for (const key of rule.exports) {
      const declared = manifest.exports?.[key];
      const target = typeof declared === 'string' ? declared : declared?.import;
      if (typeof target === 'string' && target.startsWith('./') && !target.split('/').includes('..')
        && span.literal.text === `/node_modules/${rule.package}/${target.slice(2)}`) {
        return key === '.' ? rule.package : rule.package + key.slice(1);
      }
    }
  } catch { /* No readable public contract: computed loading remains rejected. */ }
}

export function checkArchitecture(rootInput: string, owners?: readonly Owner[]): string[] {
  const root = canonical(path.resolve(rootInput));
  const errors: string[] = [];
  const selected = (owner: Owner) => !owners || owners.includes(owner);
  const gitFiles = (patterns: string[]) => execFileSync('git', ['-C', root, 'ls-files', '-z', '-co', '--exclude-standard', '--', ...patterns], { encoding: 'buffer', env: gitEnvironment }).toString().split('\0').filter(Boolean);
  const files = gitFiles(['*.ts', '*.tsx', '*.cts', '*.mts'])
    .map(p => path.resolve(root, p)).filter(p => sourceExt.has(path.extname(p)) && !p.split(path.sep).some(part => ignoredParts.has(part)) && existsSync(p));
  for (const file of files) if (selected(ownerOf(root,file)) && ownerOf(root,canonical(file)) !== ownerOf(root,file)) errors.push(`${path.relative(root,file)}: source symlink resolves into ${ownerOf(root,canonical(file))}`);
  const fileOwner = new Map(files.map(f => [f, ownerOf(root, f)]));
  const manifests = gitFiles(['*/package.json', '**/package.json']).map(p => path.resolve(root,p)).filter(p => !p.split(path.sep).some(part => ignoredParts.has(part)) && existsSync(p));
  const packageNames = new Map<string,{owner:Owner;dir:string}>();
  const fixtureImports = new Map<string, FixtureImport[]>();
  for (const mf of manifests) {
    let value: {name?:string}; try { value=JSON.parse(readFileSync(mf,'utf8')); } catch { continue; }
    if (value.name) {
      const expectedRoot = packageRoots[value.name];
      if (expectedRoot && canonical(path.dirname(mf)) !== canonical(path.resolve(root, expectedRoot))) {
        if (selected(ownerOf(root,mf))) errors.push(`${path.relative(root,mf)}: public package identity ${value.name} belongs at ${expectedRoot}`);
        continue;
      }
      if (packageNames.has(value.name) && selected(ownerOf(root,mf))) errors.push(`${path.relative(root,mf)}: duplicate package identity ${value.name}`);
      packageNames.set(value.name,{owner:ownerOf(root,path.dirname(mf)),dir:path.dirname(mf)});
    }
  }
  for (const mf of manifests) {
    const manifestOwner=ownerOf(root,path.dirname(mf));
    if (!selected(manifestOwner)) continue;
    if (!['engine','record','agent','onboarding','connect','shared','composition'].includes(manifestOwner)) continue;
    const policyPath = path.join(path.dirname(mf), 'architecture.json');
    if (existsSync(policyPath)) {
      try {
        if (canonical(policyPath) !== policyPath) throw new Error('symlinked policy');
        const policy = JSON.parse(readFileSync(policyPath, 'utf8'));
        if (policy.schema !== 1 || Object.keys(policy).sort().join(',') !== 'fixtureImports,schema'
          || !Array.isArray(policy.fixtureImports) || policy.fixtureImports.length > 16) throw new Error('invalid policy');
        for (const rule of policy.fixtureImports) {
          if (!rule || Object.keys(rule).sort().join(',') !== 'env,exports,file,function,package'
            || typeof rule.file !== 'string' || !rule.file.startsWith('test/')
            || rule.file.includes('\\') || rule.file.split('/').some((part: string) => !part || part === '.' || part === '..')
            || !/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(rule.function) || !/^[A-Z][A-Z0-9_]{0,79}$/.test(rule.env)
            || typeof rule.package !== 'string' || !Array.isArray(rule.exports) || !rule.exports.length || rule.exports.length > 16
            || rule.exports.some((entry: unknown) => typeof entry !== 'string' || !manifestExports(root, rule.package, packageNames).has(entry))) throw new Error('invalid fixture rule');
          const file = path.resolve(path.dirname(mf), rule.file);
          fixtureImports.set(file, [...(fixtureImports.get(file) ?? []), rule]);
        }
      } catch {
        errors.push(`${path.relative(root,policyPath)}: invalid bounded fixture-import policy`);
      }
    }
    let manifest: { dependencies?: Record<string,string>; devDependencies?: Record<string,string>; optionalDependencies?: Record<string,string>; peerDependencies?: Record<string,string> };
    try { manifest = JSON.parse(readFileSync(mf, 'utf8')); } catch { continue; }
    const dependencies = [manifest.dependencies, manifest.devDependencies, manifest.optionalDependencies, manifest.peerDependencies]
      .flatMap(group => Object.entries(group ?? {}));
    for (const [dep, version] of dependencies) {
      const npmAlias = /^npm:((?:@[^/]+\/)?[^@]+)(?:@.*)?$/.exec(version)?.[1];
      if (npmAlias && (packageOwners[npmAlias] || packageNames.has(npmAlias))) {
        errors.push(`${path.relative(root,mf)}: repository dependency ${npmAlias} must use its canonical public name, not npm alias ${dep}`);
      }
      const declared = packageOwners[dep] ?? packageNames.get(dep)?.owner;
      if (declared && !allowedPackage(root, mf, manifestOwner, declared, dep)) errors.push(`${path.relative(root, mf)}: dependency ${dep} violates ${manifestOwner} -> ${declared}`);
      if (/\.tgz(?:$|[?#])|\.tar\.gz(?:$|[?#])/.test(version) && !declared) errors.push(`${path.relative(root,mf)}: tarball dependency ${dep} is not a declared public package`);
      if (!version.startsWith('file:') && !version.startsWith('workspace:')) continue;
      let targetPath: string | undefined;
      if (version.startsWith('file:') && !version.endsWith('.tgz') && !version.endsWith('.tar.gz')) targetPath = path.resolve(path.dirname(mf), version.slice(5));
      if (version.startsWith('workspace:')) {
        const found = packageNames.get(dep); if (found) targetPath = found.dir;
      }
      if (!targetPath) {
        if (!declared) errors.push(`${path.relative(root,mf)}: local dependency ${dep} (${version}) is not a declared public package`);
        continue;
      }
      const realTarget=canonical(targetPath), relative=path.relative(root,realTarget);
      if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
        errors.push(`${path.relative(root,mf)}: local dependency ${dep} resolves outside the repository`); continue;
      }
      if (dep === '@apparent/native-host' && realTarget !== canonical(path.resolve(root, packageRoots[dep]))) {
        errors.push(`${path.relative(root,mf)}: local dependency ${dep} must target its canonical public package root`); continue;
      }
      const targetManifest=path.join(realTarget,'package.json');
      if (!existsSync(targetManifest)) {
        const targetOwner=ownerOf(root,realTarget);
        errors.push(`${path.relative(root,mf)}: local dependency ${dep} points into ${targetOwner} source, not a declared package root`);
        continue;
      }
      try {
        const actualName=(JSON.parse(readFileSync(targetManifest,'utf8')) as {name?:string}).name;
        const actualOwner=ownerOf(root,realTarget);
        if (actualName !== dep) errors.push(`${path.relative(root,mf)}: local dependency ${dep} targets package ${actualName ?? '(unnamed)'}`);
        if (packageOwners[dep] && actualOwner !== packageOwners[dep]) errors.push(`${path.relative(root,mf)}: local dependency ${dep} targets ${actualOwner}, expected ${packageOwners[dep]}`);
        if (!allowedPackage(root, mf, manifestOwner, actualOwner, dep)) errors.push(`${path.relative(root,mf)}: local dependency ${dep} violates ${manifestOwner} -> ${actualOwner}`);
      } catch { errors.push(`${path.relative(root,mf)}: cannot read target package manifest for ${dep}`); }
    }
  }
  for (const file of files) {
    const from = fileOwner.get(file)!;
    if (!selected(from)) continue;
    if (!product.includes(from as typeof product[number]) && from !== 'connect' && from !== 'shared' && from !== 'composition') continue;
    const text = readFileSync(file, 'utf8');
    const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const scope = configScope(root,file);
    const inspect = (spec: string, line: number) => {
      for (const target of aliasTargets(spec, scope.aliases)) {
        const resolved=canonical(target), relative=path.relative(root,resolved), to=ownerOf(root,resolved);
        if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) errors.push(`${path.relative(root,file)}:${line}: tsconfig alias resolves outside the repository (${spec})`);
        else if (to !== from) errors.push(`${path.relative(root,file)}:${line}: tsconfig alias crosses ${from} -> ${to} (${spec})`);
      }
      // baseUrl and package-import mappings can resolve bare names to source
      // without a paths entry. Use TypeScript's resolver rather than guessing.
      if (!spec.startsWith('.') && !spec.startsWith('/') && !spec.startsWith('file:') && scope.options) {
        const resolved = ts.resolveModuleName(spec, file, scope.options, ts.sys).resolvedModule;
        if (resolved && !resolved.isExternalLibraryImport) {
          const target = canonical(resolved.resolvedFileName);
          const relative = path.relative(root, target);
          if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative) || ownerOf(root, target) !== from) {
            errors.push(`${path.relative(root,file)}:${line}: resolved bare import crosses owner boundary (${spec})`);
          }
        }
      }
      const pkg = packageRef(spec, packageNames);
      if (pkg) {
        const target = packageOwners[pkg.name] ?? packageNames.get(pkg.name)!.owner;
        if (!allowedPackage(root, file, from, target, pkg.name, pkg.subpath)) errors.push(`${path.relative(root,file)}:${line}: forbidden package dependency ${from} -> ${target} (${spec})`);
        if (!manifestExports(root, pkg.name, packageNames).has(pkg.subpath)) errors.push(`${path.relative(root,file)}:${line}: private or undeclared package subpath ${spec}`);
        return;
      }
      if (spec.startsWith('.') || spec.startsWith('/') || spec.startsWith('file:')) {
        const local = resolveLocal(root,file,spec);
        const relative=path.relative(root,local), to=ownerOf(root,local);
        if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) errors.push(`${path.relative(root,file)}:${line}: local import resolves outside the repository (${spec})`);
        else if (to !== from) errors.push(`${path.relative(root,file)}:${line}: cross-owner path ${from} -> ${to} (${spec}); consume a declared public package export`);
      }
    };
    const visit = (node: ts.Node): void => {
      if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
        if (node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)) inspect(node.moduleSpecifier.text, sf.getLineAndCharacterOfPosition(node.getStart()).line + 1);
      } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference) && node.moduleReference.expression && ts.isStringLiteralLike(node.moduleReference.expression)) {
        inspect(node.moduleReference.expression.text, sf.getLineAndCharacterOfPosition(node.getStart()).line + 1);
      } else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument) && ts.isStringLiteralLike(node.argument.literal)) {
        inspect(node.argument.literal.text, sf.getLineAndCharacterOfPosition(node.getStart()).line + 1);
      } else if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
        if (node.arguments.length && ts.isStringLiteralLike(node.arguments[0])) inspect(node.arguments[0].text, sf.getLineAndCharacterOfPosition(node.getStart()).line + 1);
        else {
          const fixture = (fixtureImports.get(file) ?? []).map(rule => publicFixtureImport(root, sf, node, rule, packageNames)).find(Boolean);
          if (fixture) inspect(fixture, sf.getLineAndCharacterOfPosition(node.getStart()).line + 1);
          else if (from !== 'composition') errors.push(`${path.relative(root,file)}:${sf.getLineAndCharacterOfPosition(node.getStart()).line + 1}: computed dynamic import is not statically auditable`);
        }
      } else if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'require') {
        if (node.arguments.length && ts.isStringLiteralLike(node.arguments[0])) inspect(node.arguments[0].text, sf.getLineAndCharacterOfPosition(node.getStart()).line + 1);
        else if (from !== 'composition') errors.push(`${path.relative(root,file)}:${sf.getLineAndCharacterOfPosition(node.getStart()).line + 1}: computed require is not statically auditable`);
      }
      ts.forEachChild(node, visit);
    };
    visit(sf);
  }
  return [...new Set(errors)].sort();
}
