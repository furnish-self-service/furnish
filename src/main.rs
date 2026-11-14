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
        /// Kits: also install these optional items (comma-separated)
        #[arg(long, value_delimiter = ',', value_name = "APPS")]
        with: Vec<String>,
        /// Kits: leave out these optional items (comma-separated)
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
    /// Show or change settings (automatic collection updates in the desktop app)
    Settings {
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
        Command::Install { with, without, .. } if !with.is_empty() || !without.is_empty() => {
            anyhow::bail!("--with and --without are for kits")
        }
        Command::Uninstall { only, .. } if !only.is_empty() => anyhow::bail!("--only is for kits"),
        Command::Install { app, dry_run: true, .. } => print_plan(&catalog::find_app(&app)?, false),
        Command::Uninstall { app, dry_run: true, .. } => print_plan(&catalog::find_app(&app)?, true),
        Command::Install { app, .. } => install_app(&catalog::find_app(&app)?),
        Command::Uninstall { app, .. } => uninstall_app(&catalog::find_app(&app)?),
        Command::Collection(cmd) => collection(cmd),
        Command::Settings { update_on_launch, update_interval } => {
            let mut s = furnish::settings::load();
            if update_on_launch.is_some() || update_interval.is_some() {
                s.update_on_launch = update_on_launch.unwrap_or(s.update_on_launch);
                s.update_interval_minutes = update_interval.unwrap_or(s.update_interval_minutes);
                furnish::settings::save(&s)?;
            }
            println!("update on launch:  {}", if s.update_on_launch { "yes" } else { "no" });
            match s.update_interval_minutes {
                0 => println!("update interval:   never"),
                m => println!("update interval:   every {m} minutes"),
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
        let status = engine::status(&app.doc);
        if matches!(status, Ok(Status::Installed | Status::NeedsUpdate)) {
            installed.insert((app.collection.clone(), app.id.clone()));
        }
        let symbol = match status {
            Ok(Status::Installed) => "✓",
            Ok(Status::NeedsUpdate) => "↑",
            Ok(Status::NotInstalled) => "·",
            Ok(Status::Unavailable) => "–",
            Err(err) => {
                eprintln!("warning: could not check {}: {err:#}", app.id);
                "?"
            }
        };
        rows.push(row([symbol, &app.id, &app.collection, &app.categories.join(", "), &app.description]));
    }
    print_table(&rows);
    println!("\n✓ installed  ↑ update available  · not installed  – not available on this OS  ? unknown");

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

fn is_kit(name: &str) -> bool {
    catalog::find_app(name).is_err() && catalog::find_kit(name).is_ok()
}

fn install_app(app: &catalog::App) -> Result<()> {
    match engine::status(&app.doc)? {
        Status::Installed => {
            println!("✓ {} is already installed and up to date", app.id);
            return Ok(());
        }
        Status::Unavailable => anyhow::bail!("{} is not available on this operating system", app.id),
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
    check_items(kit, with)?;
    check_items(kit, without)?;
    if let Some(required) = kit.items.iter().find(|i| i.required && without.contains(&i.app)) {
        anyhow::bail!("'{}' is required by kit '{}'", required.app, kit.id);
    }
    let mut selected: Vec<&str> = kit.default_selection();
    selected.extend(with.iter().map(String::as_str));
    selected.retain(|app| !without.iter().any(|w| w == app));
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
