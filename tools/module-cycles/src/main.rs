//! Checks that the module graph of every library and binary target of a Cargo workspace
//! is acyclic.
//!
//! The workspace is loaded into rust-analyzer once. For each target the module graph has
//! the edges `cargo modules dependencies --no-externs --no-sysroot --no-fns --no-types
//! --no-traits --no-owns` reports: module `A` uses module `B` when `A` imports an item
//! declared in `B`, or when an item of `A` mentions a type or trait declared in `B` in a
//! signature or field. The methods of an `impl` count for the module declaring the
//! implemented type, not the module containing the `impl`. Edges between a module and
//! one of its own ancestors are dropped: that is how a Rust module tree reads, not a
//! layering cycle.
//!
//! Usage: module-cycles [--manifest-path PATH] [--dump-edges] [--timings] [CRATE...]
//!
//! Exits 0 when all graphs are acyclic, 1 when one has a cycle, 2 on other errors.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use ra_ap_cfg as cfg;
use ra_ap_hir::{self as hir, HasAttrs as _, db::HirDatabase};
use ra_ap_ide as ide;
use ra_ap_ide_db::{self as ide_db};
use ra_ap_load_cargo as load_cargo;
use ra_ap_paths as paths;
use ra_ap_project_model as project_model;

struct Args {
    manifest_path: PathBuf,
    dump_edges: bool,
    timings: bool,
    crates: Vec<String>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        manifest_path: PathBuf::from("Cargo.toml"),
        dump_edges: false,
        timings: false,
        crates: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--manifest-path" => args.manifest_path = it.next().context("--manifest-path needs a value")?.into(),
            "--dump-edges" => args.dump_edges = true,
            "--timings" => args.timings = true,
            flag if flag.starts_with("--") => bail!("unknown flag {flag}"),
            _ => args.crates.push(arg),
        }
    }
    Ok(args)
}

struct Target {
    package: String,
    kind: &'static str,
    name: String,
    root: paths::AbsPathBuf,
    edition: ide::Edition,
}

impl Target {
    fn label(&self) -> String {
        format!("{} ({} {})", self.package, self.kind, self.name)
    }
}

/// Loads the workspace; returns its library and binary targets and the database.
fn load(args: &Args, timings: &mut Timings) -> Result<(Vec<Target>, ide_db::RootDatabase, ra_ap_vfs::Vfs)> {
    let manifest = args.manifest_path.canonicalize()?;

    // What the module edges of the workspace's own crates need and what they do not:
    // - no sysroot: `std` types resolve to nothing, so only types of the workspace and its
    //   dependencies connect modules (this is what `cargo modules --no-sysroot` sees);
    // - no build scripts and no proc macros: neither changes which modules use which
    //   (cargo-modules without a sysroot has no proc-macro server either), and running
    //   build scripts costs a `cargo check` of their dependencies;
    // - dependencies stay: `Sender<Event>` only mentions `Event` once `Sender` resolves.
    let cargo_config = project_model::CargoConfig {
        all_targets: false,
        cfg_overrides: project_model::CfgOverrides {
            global: cfg::CfgDiff::new(Vec::new(), vec![cfg::CfgAtom::Flag(hir::Symbol::intern("test"))]),
            selective: Default::default(),
        },
        extra_args: vec![],
        extra_env: ide_db::FxHashMap::default(),
        extra_includes: vec![],
        features: project_model::CargoFeatures::Selected {
            features: vec![],
            no_default_features: false,
        },
        invocation_strategy: project_model::InvocationStrategy::PerWorkspace,
        no_deps: false,
        run_build_script_command: None,
        rustc_source: None,
        set_test: false,
        sysroot_src: None,
        sysroot: None,
        target_dir_config: project_model::TargetDirectoryConfig::default(),
        metadata_extra_args: vec![],
        config_path: None,
        target: None,
        wrap_rustc_in_build_scripts: true,
    };
    let progress = |_: String| {};

    let root = paths::AbsPathBuf::assert(paths::Utf8PathBuf::from_path_buf(manifest).unwrap());
    let root = project_model::ProjectManifest::discover_single(root.as_path())?;
    let workspace = project_model::ProjectWorkspace::load(root, &cargo_config, &progress)?;
    timings.lap("cargo metadata");

    let project_model::ProjectWorkspaceKind::Cargo { cargo, .. } = &workspace.kind else {
        bail!("not a Cargo workspace");
    };
    let mut targets = Vec::new();
    for package in cargo.packages().map(|p| &cargo[p]).filter(|p| p.is_member) {
        for target in package.targets.iter().map(|&t| &cargo[t]) {
            let kind = match target.kind {
                project_model::TargetKind::Lib { .. } => "lib",
                project_model::TargetKind::Bin => "bin",
                _ => continue,
            };
            targets.push(Target {
                package: package.name.clone(),
                kind,
                name: target.name.clone(),
                root: target.root.clone(),
                edition: package.edition,
            });
        }
    }

    // Name resolution of the dependency graph dominates; priming the caches does it on
    // all cores, which later queries on a single thread cannot.
    let load_config = load_cargo::LoadCargoConfig {
        load_out_dirs_from_check: false,
        prefill_caches: true,
        with_proc_macro_server: load_cargo::ProcMacroServerChoice::None,
        num_worker_threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        proc_macro_processes: 1,
    };
    let (db, vfs, _proc_macro_client) = load_cargo::load_workspace(workspace, &cargo_config.extra_env, &load_config)?;
    timings.lap("load");
    Ok((targets, db, vfs))
}

struct Timings(Option<Instant>);

impl Timings {
    fn lap(&mut self, what: &str) {
        if let Some(last) = &mut self.0 {
            eprintln!("{what}: {:?}", last.elapsed());
            *last = Instant::now();
        }
    }
}

fn find_crate(db: &ide_db::RootDatabase, vfs: &ra_ap_vfs::Vfs, target: &Target) -> Result<hir::Crate> {
    hir::Crate::all(db)
        .into_iter()
        .find(|krate| vfs.file_path(krate.root_file(db)).as_path() == Some(target.root.as_path()))
        .with_context(|| format!("no crate for {}", target.label()))
}

type Edges = BTreeSet<(String, String)>;

/// The module graph of one crate, built the way cargo-modules builds and then filters
/// its item graph.
struct ModuleGraph<'a> {
    db: &'a dyn HirDatabase,
    edition: ide::Edition,
    /// The module each function, type, trait and type alias of the crate counts for: the
    /// innermost one declaring it that is not dropped as a test module (see `is_test_cfg`).
    /// Everything else (constants, statics, macros, variants, items of other crates)
    /// is not a node cargo-modules attributes to a module.
    owner: HashMap<hir::ModuleDef, hir::Module>,
    /// Every module of the crate and the module it counts as.
    modules: HashMap<hir::Module, hir::Module>,
    edges: HashSet<(hir::Module, hir::Module)>,
}

impl<'a> ModuleGraph<'a> {
    fn new(db: &'a dyn HirDatabase, edition: ide::Edition, krate: hir::Crate) -> Self {
        let mut graph = Self {
            db,
            edition,
            owner: HashMap::new(),
            modules: HashMap::new(),
            edges: HashSet::new(),
        };
        let root = krate.root_module(db);
        let mut pending = vec![(root, root)];
        while let Some((module, kept_ancestor)) = pending.pop() {
            let counts_as = if module == root || !is_test_cfg(hir::ModuleDef::Module(module), db) {
                module
            } else {
                kept_ancestor
            };
            graph.modules.insert(module, counts_as);
            for declaration in module.declarations(db) {
                match declaration {
                    hir::ModuleDef::Module(child) => pending.push((child, counts_as)),
                    hir::ModuleDef::Function(_)
                    | hir::ModuleDef::Adt(_)
                    | hir::ModuleDef::Trait(_)
                    | hir::ModuleDef::TypeAlias(_) => {
                        graph.owner.insert(declaration, counts_as);
                    }
                    _ => {}
                }
            }
        }
        let declarations: Vec<_> = graph.owner.iter().map(|(&d, &m)| (m, d)).collect();

        for (module, counts_as) in graph.modules.clone() {
            for (_name, scope) in module.scope(db, None) {
                let hir::ScopeDef::ModuleDef(def) = scope else {
                    continue;
                };
                if def.module(db) == Some(module) {
                    continue;
                }
                let target = match def {
                    hir::ModuleDef::Module(m) => graph.modules.get(&m).copied(),
                    _ => graph.owner.get(&def).copied(),
                };
                graph.add(counts_as, target);
            }
        }
        for (module, def) in declarations {
            graph.add_item(module, def);
        }
        for imp in hir::Impl::all_in_crate(db, krate) {
            let Some(adt) = imp.self_ty(db).as_adt() else {
                continue;
            };
            let Some(&module) = graph.owner.get(&hir::ModuleDef::Adt(adt)) else {
                continue;
            };
            for item in imp.items(db) {
                match item {
                    hir::AssocItem::Function(f) => graph.add_function(module, f),
                    hir::AssocItem::TypeAlias(t) => graph.add_type(module, t.ty(db)),
                    hir::AssocItem::Const(_) => {}
                }
            }
        }
        graph
    }

    fn add(&mut self, from: hir::Module, to: Option<hir::Module>) {
        if let Some(to) = to
            && to != from
        {
            self.edges.insert((from, to));
        }
    }

    fn add_item(&mut self, module: hir::Module, def: hir::ModuleDef) {
        let db = self.db;
        match def {
            hir::ModuleDef::Function(f) => self.add_function(module, f),
            hir::ModuleDef::Adt(hir::Adt::Struct(s)) => {
                for field in s.fields(db) {
                    self.add_type(module, field.ty(db).strip_references());
                }
            }
            hir::ModuleDef::Adt(hir::Adt::Enum(e)) => {
                for variant in e.variants(db) {
                    for field in variant.fields(db) {
                        self.add_type(module, field.ty(db).strip_references());
                    }
                }
            }
            hir::ModuleDef::Adt(hir::Adt::Union(u)) => {
                for field in u.fields(db) {
                    self.add_type(module, field.ty(db).strip_references());
                }
            }
            hir::ModuleDef::TypeAlias(t) => self.add_type(module, t.ty(db)),
            _ => {}
        }
    }

    fn add_function(&mut self, module: hir::Module, f: hir::Function) {
        let db = self.db;
        for param in f.params_without_self(db) {
            self.add_type(module, param.ty().strip_references());
        }
        for param in f.assoc_fn_params(db) {
            self.add_type(module, param.ty().strip_references());
        }
        self.add_type(module, f.ret_type(db).strip_references());
    }

    /// Edges from `module` to the modules declaring the types and traits `ty` mentions.
    fn add_type(&mut self, module: hir::Module, ty: hir::Type<'_>) {
        let db = self.db;
        let mut found = Vec::new();
        ty.walk(db, |ty| {
            if let Some(adt) = ty.as_adt() {
                found.push(hir::ModuleDef::Adt(adt));
            } else if let Some(t) = ty.as_dyn_trait() {
                found.push(hir::ModuleDef::Trait(t));
            } else if let Some(traits) = ty.as_impl_traits(db) {
                found.extend(traits.map(hir::ModuleDef::Trait));
            } else if let Some(t) = ty.as_associated_type_parent_trait(db) {
                found.push(hir::ModuleDef::Trait(t));
            }
        });
        for def in found {
            let target = self.owner.get(&def).copied();
            self.add(module, target);
        }
    }

    fn path(&self, module: hir::Module) -> String {
        let krate = module.krate(self.db);
        let mut path = krate
            .display_name(self.db)
            .map(|name| name.to_string())
            .unwrap_or_default()
            .replace('-', "_");
        if let Some(relative) = hir::ModuleDef::Module(module).canonical_path(self.db, self.edition) {
            if !path.is_empty() {
                path.push_str("::");
            }
            path.push_str(&relative);
        }
        path
    }

    fn edges(&self) -> Edges {
        self.edges
            .iter()
            .map(|&(from, to)| (self.path(from), self.path(to)))
            .collect()
    }
}

/// cargo-modules drops items under a `cfg` attribute that holds when only `test` is set,
/// which includes `cfg(not(windows))`; what they use is attributed to the enclosing module.
fn is_test_cfg(def: hir::ModuleDef, db: &dyn HirDatabase) -> bool {
    let test = hir::Symbol::intern("test");
    let Some(expr) = def.attrs(db).cfgs(db) else {
        return false;
    };
    expr.fold(&|atom| match atom {
        cfg::CfgAtom::Flag(symbol) => *symbol == test,
        cfg::CfgAtom::KeyValue { .. } => false,
    })
    .unwrap_or_default()
}

/// Drops the edges between a module and its own ancestors.
fn layering_edges(edges: &Edges) -> BTreeMap<&str, Vec<&str>> {
    let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (from, to) in edges {
        let is_ancestor =
            |outer: &str, inner: &str| inner.strip_prefix(outer).is_some_and(|rest| rest.starts_with("::"));
        if is_ancestor(from, to) || is_ancestor(to, from) {
            continue;
        }
        graph.entry(from).or_default().push(to);
        graph.entry(to).or_default();
    }
    graph
}

/// Some cycle of the graph, as the modules on it in order.
fn find_cycle<'a>(graph: &BTreeMap<&'a str, Vec<&'a str>>) -> Option<Vec<&'a str>> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        OnPath,
        Done,
    }
    let mut state: HashMap<&str, State> = HashMap::new();
    for &start in graph.keys() {
        if state.contains_key(start) {
            continue;
        }
        let mut path = vec![(start, 0usize)];
        state.insert(start, State::OnPath);
        while let Some(&(node, next)) = path.last() {
            if let Some(&succ) = graph[node].get(next) {
                path.last_mut().unwrap().1 += 1;
                match state.get(succ) {
                    None => {
                        state.insert(succ, State::OnPath);
                        path.push((succ, 0));
                    }
                    Some(State::OnPath) => {
                        let at = path.iter().position(|&(n, _)| n == succ).unwrap();
                        return Some(path[at..].iter().map(|&(n, _)| n).collect());
                    }
                    Some(State::Done) => {}
                }
            } else {
                state.insert(node, State::Done);
                path.pop();
            }
        }
    }
    None
}

fn run() -> Result<bool> {
    let args = parse_args()?;
    let mut timings = Timings(args.timings.then(Instant::now));
    let (targets, db, vfs) = load(&args, &mut timings)?;
    if let Some(unknown) = args
        .crates
        .iter()
        .find(|name| !targets.iter().any(|t| &t.package == *name))
    {
        bail!("no library or binary target in package {unknown}");
    }

    let mut acyclic = true;
    hir::attach_db(&db, || -> Result<()> {
        for target in &targets {
            if !args.crates.is_empty() && !args.crates.contains(&target.package) {
                continue;
            }
            let krate = find_crate(&db, &vfs, target)?;
            let edges = ModuleGraph::new(&db, target.edition, krate).edges();
            if args.dump_edges {
                println!("== {}", target.label());
                for (from, to) in &edges {
                    println!("{from} -> {to}");
                }
            }
            if let Some(cycle) = find_cycle(&layering_edges(&edges)) {
                acyclic = false;
                println!("CYCLE in {}:", target.label());
                for module in cycle {
                    println!("  {module}");
                }
            }
        }
        Ok(())
    })?;
    timings.lap("analysis");
    Ok(acyclic)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("module-cycles: {error:#}");
            ExitCode::from(2)
        }
    }
}
