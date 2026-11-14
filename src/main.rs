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
    /// Install an app, or bring it up to date
    Install {
        /// App name, or collection/app if the name is ambiguous
        app: String,
        /// Only show what would happen
        #[arg(long)]
        dry_run: bool,
    },
    /// Uninstall an app
    Uninstall {
        /// App name, or collection/app if the name is ambiguous
        app: String,
        /// Only show what would happen
        #[arg(long)]
        dry_run: bool,
    },
    /// Manage app collections (git repositories with apps)
    #[command(subcommand)]
    Collection(CollectionCommand),
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
        Err(err) => {
            eprintln!("✗ {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::List => list(),
        Command::Install { app, dry_run: true } => print_plan(&catalog::find_app(&app)?, false),
        Command::Uninstall { app, dry_run: true } => print_plan(&catalog::find_app(&app)?, true),
        Command::Install { app, .. } => {
            let app = catalog::find_app(&app)?;
            match engine::status(&app.doc)? {
                Status::Installed => {
                    println!("✓ {} is already installed and up to date", app.id);
                    return Ok(());
                }
                Status::Unavailable => anyhow::bail!("{} is not available on this operating system", app.id),
                Status::NotInstalled | Status::NeedsUpdate => {}
            }
            println!("⟳ Installing {}…", app.id);
            engine::apply(&app.doc)?;
            println!("✓ Installed {}", app.id);
            Ok(())
        }
        Command::Uninstall { app, .. } => {
            let app = catalog::find_app(&app)?;
            if matches!(engine::status(&app.doc)?, Status::NotInstalled | Status::Unavailable) {
                println!("✓ {} is not installed", app.id);
                return Ok(());
            }
            println!("⟳ Uninstalling {}…", app.id);
            engine::remove(&app.doc)?;
            println!("✓ Uninstalled {}", app.id);
            Ok(())
        }
        Command::Collection(cmd) => collection(cmd),
    }
}

fn list() -> Result<()> {
    let apps = catalog::all_apps()?;
    if apps.is_empty() {
        println!("No apps yet. Add a collection with `furnish collection add <git-url>`.");
        return Ok(());
    }
    let mut rows = vec![row(["", "App", "Collection", "Categories", "Description"])];
    for app in &apps {
        let symbol = match engine::status(&app.doc) {
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
