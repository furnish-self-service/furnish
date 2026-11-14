mod catalog;
mod engine;

use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use engine::Status;

/// Self-service app catalog: install apps your organization curated in git.
#[derive(Parser)]
#[command(version)]
struct Cli {
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
    },
    /// Uninstall an app
    Uninstall {
        /// App name, or collection/app if the name is ambiguous
        app: String,
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
        Command::Install { app } => {
            let app = catalog::find_app(&app)?;
            if engine::status(&app.doc)? == Status::Installed {
                println!("✓ {} is already installed and up to date", app.id);
                return Ok(());
            }
            println!("⟳ Installing {}…", app.id);
            engine::apply(&app.doc)?;
            println!("✓ Installed {}", app.id);
            Ok(())
        }
        Command::Uninstall { app } => {
            let app = catalog::find_app(&app)?;
            if engine::status(&app.doc)? == Status::NotInstalled {
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
            Err(err) => {
                eprintln!("warning: could not check {}: {err:#}", app.id);
                "?"
            }
        };
        rows.push(row([symbol, &app.id, &app.collection, &app.categories.join(", "), &app.description]));
    }
    print_table(&rows);
    println!("\n✓ installed  ↑ update available  · not installed  ? unknown");
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
