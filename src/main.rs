use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use furnish::{catalog, engine, engine::Status};

/// Self-service app catalog: install apps your organization curated in git.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Show what DSC and the package manager are doing
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List all apps and their status
    #[command(visible_alias = "ls")]
    List,
    /// Install an app or a kit, or bring it up to date
    Install {
        /// App or kit name, or collection/name if the name is ambiguous
        app: String,
        /// Only show what would happen
        #[arg(long)]
        dry_run: bool,
        /// Also install these optional kit items or app dependencies (comma-separated)
        #[arg(long, value_delimiter = ',', value_name = "APPS")]
        with: Vec<String>,
        /// Leave out these optional kit items or app dependencies (comma-separated)
        #[arg(long, value_delimiter = ',', value_name = "APPS")]
        without: Vec<String>,
    },
    /// Uninstall an app, or apps of a kit
    Uninstall {
        /// App or kit name, or collection/name if the name is ambiguous
        app: String,
        /// Only show what would happen
        #[arg(long)]
        dry_run: bool,
        /// Kits: uninstall exactly these items (default: installed items no other installed kit needs)
        #[arg(long, value_delimiter = ',', value_name = "APPS")]
        only: Vec<String>,
    },
    /// Manage app collections (git repositories with apps)
    #[command(subcommand)]
    Collection(CollectionCommand),
    /// Install available updates (apps with automatic updates on; --all: every app)
    Upgrade {
        /// Also update apps with automatic updates turned off
        #[arg(long)]
        all: bool,
    },
    /// Show or change settings (automatic updates in the desktop app)
    Settings {
        /// Turn automatic updates on for an app again
        #[arg(long, value_name = "APP")]
        auto_update: Vec<String>,
        /// Turn automatic updates off for an app
        #[arg(long, value_name = "APP")]
        no_auto_update: Vec<String>,
        /// Update collections when the desktop app starts
        #[arg(long)]
        update_on_launch: Option<bool>,
        /// Update collections every N minutes while the desktop app runs (0: never)
        #[arg(long, value_name = "MINUTES")]
        update_interval: Option<u32>,
    },
}

#[derive(Subcommand)]
enum CollectionCommand {
    /// Add a collection from a git URL
    Add {
        url: String,
        /// Defaults to the last part of the URL
        #[arg(long)]
        name: Option<String>,
    },
    /// List collections
    #[command(visible_alias = "ls")]
    List,
    /// Pull the latest version of one or all collections
    Update { name: Option<String> },
    /// Remove a collection (installed apps stay installed)
    Remove { name: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.verbose {
        engine::capture_logs(print_log_line);
    }
    let result = engine::init(&catalog::data_dir().join("resources")).and_then(|()| run(cli.command));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.is::<engine::Cancelled>() => {
            eprintln!("⊘ Cancelled");
            ExitCode::from(130)
        }
        Err(err) => {
            eprintln!("✗ {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::List => list(),
        // Kits: names that are no app but a kit.
        Command::Install { app, dry_run, with, without } if is_kit(&app) => {
            install_kit(&catalog::find_kit(&app)?, &with, &without, dry_run)
        }
        Command::Uninstall { app, dry_run, only } if is_kit(&app) => uninstall_kit(&catalog::find_kit(&app)?, &only, dry_run),
        Command::Uninstall { only, .. } if !only.is_empty() => anyhow::bail!("--only is for kits"),
        Command::Install { app, dry_run: true, with, without } => {
            let app = catalog::find_app(&app)?;
            for dep in chosen_dependencies(&app, &with, &without)? {
                if !matches!(engine::status(&dep.doc)?, Status::Installed | Status::NeedsUpdate | Status::Unavailable) {
                    println!("First, dependency {}:", dep.id);
                    print_plan(&dep, false)?;
                }
            }
            print_plan(&app, false)
        }
        Command::Uninstall { app, dry_run: true, .. } => print_plan(&catalog::find_app(&app)?, true),
        Command::Install { app, with, without, .. } => {
            install_with_dependencies(&catalog::find_app(&app)?, &with, &without, &mut Vec::new())
        }
        Command::Uninstall { app, .. } => uninstall_app(&catalog::find_app(&app)?),
        Command::Collection(cmd) => collection(cmd),
        Command::Upgrade { all } => upgrade(all),
        Command::Settings { update_on_launch, update_interval, auto_update, no_auto_update } => {
            let mut s = furnish::settings::load();
            let key = |name: &str| catalog::find_app(name).map(|a| format!("{}/{}", a.collection, a.id));
            for name in &no_auto_update {
                let key = key(name)?;
                if !s.no_auto_update.contains(&key) {
                    s.no_auto_update.push(key);
                }
            }
            for name in &auto_update {
                let key = key(name)?;
                s.no_auto_update.retain(|k| k != &key);
            }
            if update_on_launch.is_some() || update_interval.is_some() || !auto_update.is_empty() || !no_auto_update.is_empty() {
                s.update_on_launch = update_on_launch.unwrap_or(s.update_on_launch);
                s.update_interval_minutes = update_interval.unwrap_or(s.update_interval_minutes);
                furnish::settings::save(&s)?;
            }
            println!("update on launch:  {}", if s.update_on_launch { "yes" } else { "no" });
            match s.update_interval_minutes {
                0 => println!("update interval:   never"),
                m => println!("update interval:   every {m} minutes"),
            }
            match s.no_auto_update.as_slice() {
                [] => println!("app updates:       automatic for all apps"),
                apps => println!("app updates:       automatic, except {}", apps.join(", ")),
            }
            Ok(())
        }
    }
}

fn list() -> Result<()> {
    let apps = catalog::all_apps()?;
    if apps.is_empty() {
        println!("No apps yet. Add a collection with `furnish collection add <git-url>`.");
        return Ok(());
    }
    let mut rows = vec![row(["", "App", "Collection", "Categories", "Description"])];
    let mut installed = std::collections::HashSet::new();
    for app in &apps {
        let check = engine::check(&app.doc);
        if matches!(check, Ok(engine::Check { status: Status::Installed | Status::NeedsUpdate, .. })) {
            installed.insert((app.collection.clone(), app.id.clone()));
        }
        let mut description = app.description.clone();
        let symbol = match check {
            Ok(c) => match c.status {
                Status::Installed => "✓",
                Status::NeedsUpdate => "↑",
                Status::NotInstalled => "·",
                Status::Unavailable => "–",
                Status::Blocked => {
                    let needs = c.reason.unwrap_or_default();
                    let deps = catalog::default_selection(&app.dependencies);
                    description = if deps.is_empty() {
                        format!("needs {needs}")
                    } else {
                        format!("needs {needs}; installing it installs {} first", deps.join(", "))
                    };
                    "!"
                }
            },
            Err(err) => {
                eprintln!("warning: could not check {}: {err:#}", app.id);
                "?"
            }
        };
        rows.push(row([symbol, &app.id, &app.collection, &app.categories.join(", "), &description]));
    }
    print_table(&rows);
    println!("\n✓ installed  ↑ update available  · not installed  – not available on this OS  ! needs a missing tool  ? unknown");

    let kits = catalog::all_kits()?;
    if !kits.is_empty() {
        println!("\nKits (install with `furnish install <kit>`, see --with/--without):\n");
        let mut rows = vec![row(["", "Kit", "Collection", "Items", "Description"])];
        for kit in &kits {
            let have = kit.items.iter().filter(|i| installed.contains(&(kit.collection.clone(), i.app.clone()))).count();
            let symbol = if have == kit.items.len() { "✓" } else if have > 0 { "◐" } else { "·" };
            let items = format!("{have} of {} installed", kit.items.len());
            rows.push(row([symbol, &kit.id, &kit.collection, &items, &kit.description]));
        }
        print_table(&rows);
    }
    Ok(())
}

fn collection(cmd: CollectionCommand) -> Result<()> {
    match cmd {
        CollectionCommand::Add { url, name } => {
            let name = name.unwrap_or_else(|| catalog::name_from_url(&url));
            let collection = catalog::add(&url, &name)?;
            let apps = collection.apps().len();
            println!("✓ Added '{name}' at revision {} with {apps} app(s)", collection.revision()?);
            println!("⚠ Apps run with administrator rights. Only add collections from authors you trust.");
        }
        CollectionCommand::List => {
            let mut rows = vec![row(["Name", "Apps", "Revision", "URL"])];
            for c in catalog::collections()? {
                let (apps, rev, url) = (c.apps().len().to_string(), c.revision()?, c.url()?);
                rows.push(row([&c.name, &apps, &rev, &url]));
            }
            print_table(&rows);
        }
        CollectionCommand::Update { name } => {
            let collections = match name {
                Some(name) => vec![catalog::get(&name)?],
                None => catalog::collections()?,
            };
            for c in collections {
                match c.update()? {
                    (old, new) if old == new => println!("✓ {} is up to date ({new})", c.name),
                    (old, new) => println!("✓ Updated {} from {old} to {new}", c.name),
                }
            }
        }
        CollectionCommand::Remove { name } => {
            catalog::remove(&name)?;
            println!("✓ Removed '{name}'. Apps installed from it stay installed.");
        }
    }
    Ok(())
}

/// Prints each step as it starts and finishes, with duration and result.
fn print_progress() -> impl FnMut(engine::Progress) {
    use engine::{Action, Progress};
    let mut steps = Vec::new();
    move |event| match event {
        Progress::Planned { steps: planned } => steps = planned,
        Progress::Started { index, .. } => {
            let step: &engine::Step = &steps[index];
            println!("  ▸ [{}/{}] {} …", index + 1, steps.len(), step.name);
        }
        Progress::Finished { index, duration_ms, outcome, .. } => {
            let step = &steps[index];
            let secs = duration_ms as f64 / 1000.0;
            match (&outcome.error, outcome.action) {
                _ if outcome.cancelled => println!("  ⊘ {} cancelled after {secs:.1}s", step.name),
                (Some(err), _) => println!("  ✗ {} failed after {secs:.1}s: {}", step.name, err.lines().next().unwrap_or_default()),
                (None, Action::Skipped) => println!("  – {} skipped: not for this system", step.name),
                (None, Action::Nothing) => println!("  ✓ {} already as described ({secs:.1}s)", step.name),
                (None, action) => {
                    let what = match action { Action::Install => "installed", Action::Remove => "removed", _ => "changed" };
                    println!("  ✓ {} {what} ({secs:.1}s)", step.name);
                }
            }
        }
    }
}

fn upgrade(all: bool) -> Result<()> {
    let settings = furnish::settings::load();
    let (mut updated, mut skipped) = (0, Vec::new());
    for app in catalog::all_apps()? {
        if engine::status(&app.doc).ok() != Some(Status::NeedsUpdate) {
            continue;
        }
        let key = format!("{}/{}", app.collection, app.id);
        if !all && settings.no_auto_update.contains(&key) {
            skipped.push(key);
            continue;
        }
        install_app(&app)?;
        updated += 1;
    }
    if updated == 0 {
        println!("✓ Nothing to update");
    }
    if !skipped.is_empty() {
        println!("Not updated (automatic updates off): {}. Use --all to update them too.", skipped.join(", "));
    }
    Ok(())
}

fn is_kit(name: &str) -> bool {
    catalog::find_app(name).is_err() && catalog::find_kit(name).is_ok()
}

fn install_app(app: &catalog::App) -> Result<()> {
    install_with_dependencies(app, &[], &[], &mut Vec::new())
}

/// The app's dependencies to install: required ones, selected optional ones, `with`, minus `without`.
fn chosen_dependencies(app: &catalog::App, with: &[String], without: &[String]) -> Result<Vec<catalog::App>> {
    let names = catalog::choose(&app.dependencies, with, without, &format!("the dependencies of {}", app.id))?;
    let siblings = catalog::get(&app.collection)?.apps();
    names
        .into_iter()
        .map(|name| {
            siblings.iter().find(|a| a.id == name).cloned().ok_or_else(|| {
                anyhow::anyhow!("{} depends on {name}, which is not an app in collection {}", app.id, app.collection)
            })
        })
        .collect()
}

/// Install the app's chosen dependencies first (theirs with their defaults), then the app.
fn install_with_dependencies(app: &catalog::App, with: &[String], without: &[String], path: &mut Vec<String>) -> Result<()> {
    if path.contains(&app.id) {
        anyhow::bail!("dependencies form a cycle: {} → {}", path.join(" → "), app.id);
    }
    path.push(app.id.clone());
    for dep in chosen_dependencies(app, with, without)? {
        // Installed (even if outdated) is enough; dependencies not for this OS don't apply.
        if !matches!(engine::status(&dep.doc)?, Status::Installed | Status::NeedsUpdate | Status::Unavailable) {
            println!("{} needs {} first", app.id, dep.id);
            install_with_dependencies(&dep, &[], &[], path)?;
        }
    }
    path.pop();
    let check = engine::check(&app.doc)?;
    match check.status {
        Status::Installed => {
            println!("✓ {} is already installed and up to date", app.id);
            return Ok(());
        }
        Status::Unavailable => anyhow::bail!("{} is not available on this operating system", app.id),
        Status::Blocked => anyhow::bail!("{} needs {}", app.id, check.reason.unwrap_or_default()),
        Status::NotInstalled | Status::NeedsUpdate => {}
    }
    println!("⟳ Installing {}…", app.id);
    engine::run(&app.doc, false, print_progress())?;
    println!("✓ Installed {}", app.id);
    Ok(())
}

fn uninstall_app(app: &catalog::App) -> Result<()> {
    if matches!(engine::status(&app.doc)?, Status::NotInstalled | Status::Unavailable) {
        println!("✓ {} is not installed", app.id);
        return Ok(());
    }
    println!("⟳ Uninstalling {}…", app.id);
    engine::run(&app.doc, true, print_progress())?;
    println!("✓ Uninstalled {}", app.id);
    Ok(())
}

/// The kit's apps from its collection, by id.
fn kit_apps(kit: &catalog::Kit) -> Result<Vec<catalog::App>> {
    let collection = catalog::get(&kit.collection)?;
    Ok(collection.apps().into_iter().filter(|a| kit.items.iter().any(|i| i.app == a.id)).collect())
}

fn check_items(kit: &catalog::Kit, names: &[String]) -> Result<()> {
    for name in names {
        if !kit.items.iter().any(|i| &i.app == name) {
            anyhow::bail!("'{name}' is not part of kit '{}'", kit.id);
        }
    }
    Ok(())
}

fn install_kit(kit: &catalog::Kit, with: &[String], without: &[String], dry_run: bool) -> Result<()> {
    let selected = catalog::choose(&kit.items, with, without, &format!("kit '{}'", kit.id))?;
    let apps = kit_apps(kit)?;
    for item in &kit.items {
        if !selected.contains(&item.app.as_str()) {
            println!("– {} (not selected)", item.app);
            continue;
        }
        let app = apps.iter().find(|a| a.id == item.app).expect("kit items are validated");
        if dry_run { print_plan(app, false)?; } else { install_app(app)?; }
    }
    Ok(())
}

fn uninstall_kit(kit: &catalog::Kit, only: &[String], dry_run: bool) -> Result<()> {
    check_items(kit, only)?;
    let apps = kit_apps(kit)?;
    let selected: Vec<String> = if only.is_empty() {
        let kits = catalog::all_kits()?;
        // Status of every app any kit mentions, to tell which other kits are installed.
        let mut installed = std::collections::HashSet::new();
        for k in &kits {
            for a in kit_apps(k)? {
                if matches!(engine::status(&a.doc), Ok(Status::Installed | Status::NeedsUpdate)) {
                    installed.insert((k.collection.clone(), a.id.clone()));
                }
            }
        }
        let is_installed = |c: &str, a: &str| installed.contains(&(c.to_string(), a.to_string()));
        catalog::default_uninstall(kit, &kits, &is_installed).into_iter().map(String::from).collect()
    } else {
        only.to_vec()
    };
    if selected.is_empty() {
        println!("Nothing to uninstall: no item of {} is installed, or other installed kits need them.", kit.id);
    }
    // Reverse kit order, so later items (which may build on earlier ones) go first.
    for item in kit.items.iter().rev().filter(|i| selected.contains(&i.app)) {
        let app = apps.iter().find(|a| a.id == item.app).expect("kit items are validated");
        if dry_run { print_plan(app, true)?; } else { uninstall_app(app)?; }
    }
    Ok(())
}

fn print_plan(app: &catalog::App, uninstall: bool) -> Result<()> {
    use engine::Action;
    let steps = engine::plan(&app.doc, uninstall)?;
    println!("{} {} would:", if uninstall { "Uninstalling" } else { "Installing" }, app.id);
    for step in &steps {
        let label = match step.action {
            Action::Install => "+ install",
            Action::Remove => "- remove",
            Action::Change => "~ change",
            Action::Nothing => "= nothing to do",
            Action::Skipped => "  skipped",
        };
        println!("\n  {label:<16} {}  ({})", step.name, step.kind);
        for detail in &step.details {
            println!("  {:<16} {detail}", "");
        }
        for c in &step.changes {
            println!("  {:<16} {}: {} → {}", "", c.property, c.before, c.after);
        }
    }
    Ok(())
}

fn print_log_line(line: engine::LogLine) {
    use std::io::IsTerminal;
    let color = match line.level {
        "error" => "31",
        "warn" => "33",
        "info" => "36",
        _ => "2", // resource output: dimmed
    };
    if std::io::stderr().is_terminal() {
        eprintln!("\x1b[{color}m{:>6} │ {}\x1b[0m", line.level, line.message);
    } else {
        eprintln!("{:>6} │ {}", line.level, line.message);
    }
}

fn row<const N: usize>(cells: [&str; N]) -> Vec<String> {
    cells.iter().map(|c| c.to_string()).collect()
}

fn print_table(rows: &[Vec<String>]) {
    let widths: Vec<usize> = (0..rows[0].len())
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    for r in rows {
        let line: Vec<_> = r.iter().zip(&widths).map(|(cell, w)| format!("{cell:<w$}")).collect();
        println!("{}", line.join("  ").trim_end());
    }
}
