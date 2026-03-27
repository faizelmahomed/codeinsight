use std::collections::HashMap;
use crate::analyzer::FileAnalysis;
use crate::depgraph::{DeadCode, DepGraph};
use crate::git::GitContext;
use crate::lang::lang_abbrev;
use crate::locations::KeyLocations;
use crate::models::DataLayer;
use crate::project::ProjectContext;
use crate::scanner::{ScanResults, TestMap};
use crate::tooling::ToolingContext;

pub struct AggregatedStats {
    pub files: u32,
    pub total_lines: u32,
    pub by_language: HashMap<String, LangStats>,
}

#[derive(Default)]
pub struct LangStats {
    pub files: u32,
    pub lines: u32,
    pub functions: u32,
    pub classes: u32,
    pub complexity: u32,
}

pub fn format_compact(
    stats: &AggregatedStats,
    file_metrics: &HashMap<String, FileAnalysis>,
    dep_graph: &DepGraph,
    dead_code: &DeadCode,
    duplicates: &[(String, Vec<(String, String)>)],
    project: &ProjectContext,
    git: &GitContext,
    tooling: &ToolingContext,
    scans: &ScanResults,
    test_map: &TestMap,
    data_layer: &DataLayer,
    key_locations: &KeyLocations,
) -> String {
    let mut out = String::with_capacity(4096);

    // Project line
    if let Some(ref name) = project.name {
        let ver = project.version.as_deref().unwrap_or("");
        let desc = project.description.as_deref().unwrap_or("");
        if !desc.is_empty() {
            out.push_str(&format!("Project: {} v{} ({}) - {}\n", name, ver, project.project_type, desc));
        } else {
            out.push_str(&format!("Project: {} v{} ({})\n", name, ver, project.project_type));
        }
    }

    // README excerpt — project purpose/intent
    if let Some(ref excerpt) = project.readme_excerpt {
        if !excerpt.is_empty() {
            out.push_str(&format!("About: {}\n", excerpt));
        }
    }

    // Stack
    let node_builtins: std::collections::HashSet<&str> = [
        "fs", "path", "os", "util", "crypto", "http", "https", "url", "stream",
        "events", "child_process", "assert", "buffer", "querystring", "zlib",
        "net", "tls", "dns", "cluster", "readline", "worker_threads",
        "node:fs", "node:path", "node:os", "node:util", "node:crypto",
        "node:http", "node:https", "node:url", "node:stream", "node:events",
        "node:child_process", "node:assert", "node:buffer", "node:test",
    ].iter().copied().collect();

    let known_services: &[(&str, &str)] = &[
        ("stripe", "Stripe"), ("@stripe", "Stripe"),
        ("redis", "Redis"), ("ioredis", "Redis"),
        ("prisma", "Prisma"), ("@prisma", "Prisma"),
        ("drizzle-orm", "Drizzle"), ("mongoose", "MongoDB"), ("mongodb", "MongoDB"),
        ("pg", "PostgreSQL"), ("mysql2", "MySQL"),
        ("@aws-sdk", "AWS"), ("aws-sdk", "AWS"),
        ("firebase", "Firebase"), ("@supabase", "Supabase"),
        ("socket.io", "Socket.IO"), ("graphql", "GraphQL"), ("@apollo", "Apollo"),
        ("tailwindcss", "Tailwind"), ("@sentry", "Sentry"), ("sentry", "Sentry"),
        ("zod", "Zod"), ("trpc", "tRPC"), ("@trpc", "tRPC"),
    ];

    let mut stack_parts: Vec<String> = project.frameworks.iter().cloned().collect();
    for (prefix, label) in known_services {
        let found = dep_graph.external_imports.keys().any(|k| {
            k == *prefix || k.starts_with(&format!("{}/", prefix))
        }) || project.dependencies.iter().any(|d| d == *prefix || d.starts_with(&format!("{}/", prefix)))
          || project.dev_dependencies.iter().any(|d| d == *prefix || d.starts_with(&format!("{}/", prefix)));
        if found && !stack_parts.iter().any(|s| s == *label) {
            stack_parts.push(label.to_string());
        }
    }
    if let Some(ref orm) = data_layer.orm {
        if !stack_parts.iter().any(|s| s == orm) {
            stack_parts.push(orm.clone());
        }
    }
    if !stack_parts.is_empty() {
        out.push_str(&format!("Stack: {}\n", stack_parts.join(", ")));
    }

    // Header
    let mut langs: Vec<_> = stats.by_language.iter().collect();
    langs.sort_by(|a, b| b.1.lines.cmp(&a.1.lines));
    let lang_str: Vec<String> = langs.iter().take(4).map(|(name, data)| {
        let pct = if stats.total_lines > 0 { (data.lines as f64 / stats.total_lines as f64 * 100.0) as u32 } else { 0 };
        format!("{} {}%", lang_abbrev(name), pct)
    }).collect();
    out.push_str(&format!("{} files | {}L | {}\n", stats.files, fmt_k(stats.total_lines), lang_str.join(", ")));

    // Scripts
    let mut scripts = Vec::new();
    if let Some(dev) = project.scripts.get("dev") {
        scripts.push(format!("Dev: {}", dev));
    } else if let Some(start) = project.scripts.get("start") {
        scripts.push(format!("Run: {}", start));
    } else if let Some(run) = project.scripts.get("run") {
        scripts.push(format!("Run: {}", run));
    }
    if let Some(build) = project.scripts.get("build") {
        scripts.push(format!("Build: {}", build));
    }
    if let Some(test) = project.scripts.get("test") {
        scripts.push(format!("Test: {}", test));
    }
    if !scripts.is_empty() {
        out.push_str(&format!("{}\n", scripts.join(" | ")));
    }

    out.push('\n');

    // Env vars — show all
    let mut all_env: Vec<String> = Vec::new();
    for analysis in file_metrics.values() {
        for v in &analysis.env_vars {
            if !all_env.contains(v) {
                all_env.push(v.clone());
            }
        }
    }
    if !all_env.is_empty() {
        all_env.sort();
        out.push_str(&format!("Env: {}\n", all_env.join(", ")));
    }

    // Routes — show all
    let mut all_routes: Vec<String> = Vec::new();
    for analysis in file_metrics.values() {
        for r in &analysis.http_routes {
            if !all_routes.contains(r) {
                all_routes.push(r.clone());
            }
        }
    }
    if !all_routes.is_empty() {
        out.push_str(&format!("Routes: {}\n", all_routes.join(", ")));
    }

    // Models — show all
    if !data_layer.model_names.is_empty() {
        out.push_str(&format!("Models: {}\n", data_layer.model_names.join(", ")));
        if !data_layer.schema_files.is_empty() {
            out.push_str(&format!("Schema: {}\n", data_layer.schema_files.join(", ")));
        }
    }

    out.push('\n');

    // Key dirs
    if !key_locations.locations.is_empty() {
        let dirs: Vec<String> = key_locations.locations.iter()
            .filter(|l| l.count >= 2)
            .map(|l| format!("{}({})", l.path, l.count))
            .collect();
        if !dirs.is_empty() {
            out.push_str(&format!("Dirs: {}\n", dirs.join(", ")));
        }
    }

    // Git
    if git.is_repo {
        let mut git_parts = Vec::new();
        if let Some(ref branch) = git.branch {
            git_parts.push(format!("Branch: {}", branch));
        }
        if !git.uncommitted.is_empty() {
            git_parts.push(format!("{} uncommitted", git.uncommitted.len()));
        }
        if !git_parts.is_empty() {
            out.push_str(&format!("{}\n", git_parts.join(" | ")));
        }
        if !git.hot_files.is_empty() {
            let hot: Vec<String> = git.hot_files.iter().map(|(f, c)| {
                let fname = f.rsplit('/').next().unwrap_or(f);
                format!("{}({})", fname, c)
            }).collect();
            out.push_str(&format!("Hot: {}\n", hot.join(", ")));
        }
    }

    // TODOs — show all
    if !scans.todos.is_empty() || !scans.fixmes.is_empty() || !scans.hacks.is_empty() {
        let mut notes = Vec::new();
        for n in &scans.todos {
            let fname = n.file.rsplit('/').next().unwrap_or(&n.file);
            if n.text.is_empty() {
                notes.push(format!("TODO {}:{}", fname, n.line));
            } else {
                notes.push(format!("TODO {}:{} \"{}\"", fname, n.line, n.text));
            }
        }
        for n in &scans.fixmes {
            let fname = n.file.rsplit('/').next().unwrap_or(&n.file);
            if n.text.is_empty() {
                notes.push(format!("FIXME {}:{}", fname, n.line));
            } else {
                notes.push(format!("FIXME {}:{} \"{}\"", fname, n.line, n.text));
            }
        }
        for n in &scans.hacks {
            let fname = n.file.rsplit('/').next().unwrap_or(&n.file);
            if n.text.is_empty() {
                notes.push(format!("HACK {}:{}", fname, n.line));
            } else {
                notes.push(format!("HACK {}:{} \"{}\"", fname, n.line, n.text));
            }
        }
        out.push_str(&format!("{}\n", notes.join(" | ")));
    }

    // Tests
    if test_map.test_count > 0 || test_map.source_count > 0 {
        let pct = if test_map.source_count > 0 {
            (test_map.test_count as f64 / test_map.source_count as f64 * 100.0) as u32
        } else { 0 };
        out.push_str(&format!("Tests: {}/{} ({}%)\n", test_map.test_count, test_map.source_count, pct));
    }

    // External deps
    let mut ext_deps: Vec<_> = dep_graph.external_imports.iter()
        .filter(|(k, _)| !node_builtins.contains(k.as_str()))
        .filter(|(k, _)| !k.starts_with("@/") && !k.starts_with("./") && !k.starts_with("../"))
        .collect();
    ext_deps.sort_by(|a, b| b.1.cmp(a.1));
    if !ext_deps.is_empty() {
        let deps: Vec<String> = ext_deps.iter().take(10).map(|(k, v)| format!("{}({})", k, v)).collect();
        out.push_str(&format!("Deps: {}\n", deps.join(", ")));
    }

    // Tooling
    let mut tool_parts = Vec::new();
    if let Some(ref ts) = tooling.typescript {
        let mode = if ts.strict { "strict" } else { "standard" };
        tool_parts.push(format!("TS {}", mode));
    }
    if !tooling.linting.is_empty() {
        tool_parts.extend(tooling.linting.iter().cloned());
    }
    if tooling.has_prettier {
        tool_parts.push("Prettier".into());
    }
    if let Some(ref fw) = tooling.testing {
        tool_parts.push(fw.clone());
    }
    if !tooling.ci.is_empty() {
        tool_parts.extend(tooling.ci.iter().cloned());
    }
    if tooling.has_dockerfile {
        tool_parts.push("Docker".into());
    }
    if !tool_parts.is_empty() {
        out.push_str(&format!("Tooling: {}\n", tool_parts.join(", ")));
    }

    // Security — only show real issues
    if !scans.security.is_empty() {
        let mut by_kind: HashMap<&str, Vec<String>> = HashMap::new();
        for issue in &scans.security {
            let fname = issue.file.rsplit('/').next().unwrap_or(&issue.file);
            by_kind.entry(&issue.kind).or_default().push(format!("{}:{}", fname, issue.line));
        }
        let mut sec_parts = Vec::new();
        for (kind, locs) in &by_kind {
            let label = match *kind {
                "eval" => "eval()",
                "secret" => "hardcoded secrets",
                "sql_injection" => "SQL injection",
                _ => kind,
            };
            sec_parts.push(format!("{} in {}", label, locs.join(", ")));
        }
        out.push_str(&format!("Security: {}\n", sec_parts.join(" | ")));
    }

    out.push('\n');

    // Issues summary
    let mut issues = Vec::new();
    if !dead_code.orphaned_files.is_empty() {
        issues.push(format!("{} orphaned files", dead_code.orphaned_files.len()));
    }
    if !dead_code.unused_exports.is_empty() {
        issues.push(format!("{} unused exports", dead_code.unused_exports.len()));
    }
    if !dead_code.possibly_dead.is_empty() {
        issues.push(format!("{} single-use files", dead_code.possibly_dead.len()));
    }
    let large = file_metrics.values().filter(|a| a.stats.lines > 500).count();
    if large > 0 {
        issues.push(format!("{} files >500 lines", large));
    }
    let complex: usize = file_metrics.values().flat_map(|a| a.func_names.iter()).filter(|f| f.lines > 50).count();
    if complex > 0 {
        issues.push(format!("{} functions >50 lines", complex));
    }
    if !duplicates.is_empty() {
        issues.push(format!("{} duplicated groups", duplicates.len()));
    }
    if !issues.is_empty() {
        out.push_str(&format!("Issues: {}\n", issues.join(", ")));
    }

    // Exported API — show all for libraries/small projects
    let mut exported_fns: Vec<String> = Vec::new();
    for (path, analysis) in file_metrics {
        for func in &analysis.func_names {
            if analysis.exported_names.contains(&func.name) {
                let fname = path.rsplit('/').next().unwrap_or(path);
                exported_fns.push(format!("{}:{}:{}", fname, func.start_line, func.name));
            }
        }
    }
    if !exported_fns.is_empty() && exported_fns.len() <= 30 {
        out.push_str(&format!("Exports: {}\n", exported_fns.join(", ")));
    } else if !exported_fns.is_empty() {
        let shown: Vec<&str> = exported_fns.iter().take(15).map(|s| s.as_str()).collect();
        out.push_str(&format!("Exports: {} (+{})\n", shown.join(", "), exported_fns.len() - 15));
    }

    out.trim_end().to_string()
}

fn fmt_k(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}
