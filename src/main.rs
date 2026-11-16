use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use furnish::{catalog, engine, engine::Status};

/// Self-service app catalog: install apps your organization curated in git.
#[derive(Parser)]
// FURNISH_VERSION at build time: the package version of alpha builds (e.g. 0.1.0+alpha.42).
#[command(version = option_env!("FURNISH_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))]
struct Cli {
    /// Show what DSC and the package manager are doing
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The furnish service, which installs your organization's apps for users without
    /// administrator rights (see the `service` section of the admin policy). Without an action:
    /// run it (as root or SYSTEM; the service manager does that).
    Service {
        #[command(subcommand)]
        action: Option<ServiceAction>,
    },
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
        /// Install the previous version of the app: the one that last installed here without
        /// error, else the one before the collection's latest change to it
        #[arg(long, conflicts_with = "current")]
        previous: bool,
        /// For an app held at an earlier version: try the collection's current version again
        #[arg(long)]
        current: bool,
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
    /// Sign in to your organization's furnish address (e.g. furnish.myorg.tld) in the browser
    Login { endpoint: String },
    /// Forget the sign-in to a furnish address
    Logout { endpoint: String },
    /// Uninstall and install an app again, to apply options you changed
    Reinstall { app: String },
    /// Show or change options (DSC parameters such as userInstall) of a collection or an app
    #[command(subcommand)]
    Option(OptionCommand),
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
        /// Postpone automatic updates while the computer is busy
        #[arg(long)]
        load_aware_updates: Option<bool>,
        /// Update collections every N minutes while the desktop app runs (0: never)
        #[arg(long, value_name = "MINUTES")]
        update_interval: Option<u32>,
        #[command(flatten)]
        collections: CollectionSettingsArgs,
        /// Reports to your organization: off, errors (failures and fallbacks) or all
        #[arg(long, value_parser = ["off", "errors", "all"])]
        reporting: Option<String>,
        /// Include this computer's name in reports
        #[arg(long, value_name = "BOOL")]
        report_hostname: Option<bool>,
        /// Show collection texts in this language when they aren't translated into the app's
        /// language (a language tag such as en, de or pt-BR)
        #[arg(long, value_name = "LANGUAGE")]
        fallback_language: Option<String>,
    },
    /// Show the reports waiting to be sent to your organization, and where they go
    Reports,
    /// Check a collection (a checkout of its repository) before people install from it: every
    /// file loads, installers and icons exist, expressions evaluate; shows which resources run on
    /// Windows, macOS and Linux. Exits with 1 on errors. Installs nothing.
    Check {
        /// The collection's directory
        #[arg(default_value = ".")]
        dir: std::path::PathBuf,
    },
}

/// Settings that can differ per collection (`furnish collection settings`).
#[derive(clap::Args)]
struct CollectionSettingsArgs {
    /// Keep this many commits of a collection's history (0: all)
    #[arg(long, value_name = "COMMITS")]
    checkout_depth: Option<u32>,
    /// Download file contents only when needed: earlier versions when falling back, installer
    /// files when an app using them is installed
    #[arg(long, value_name = "BOOL")]
    blobs_on_demand: Option<bool>,
    /// When installing or updating an app fails: offer the previous version (ask), install it
    /// right away (automatic), or neither (off)
    #[arg(long, value_parser = ["ask", "automatic", "off"])]
    fallback: Option<String>,
    /// Fall back only to versions at most this many days older than the failing one (0: any)
    #[arg(long, value_name = "DAYS")]
    fallback_max_age: Option<u32>,
}

impl CollectionSettingsArgs {
    fn changes(&self) -> Vec<(&'static str, serde_json::Value)> {
        let mut changes = Vec::new();
        if let Some(v) = self.checkout_depth {
            changes.push(("checkoutDepth", v.into()));
        }
        if let Some(v) = self.blobs_on_demand {
            changes.push(("blobsOnDemand", v.into()));
        }
        if let Some(v) = &self.fallback {
            changes.push(("fallback", v.as_str().into()));
        }
        if let Some(v) = self.fallback_max_age {
            changes.push(("fallbackMaxAgeDays", v.into()));
        }
        changes
    }
}

fn print_collection_settings(s: &furnish::settings::CollectionSettings, locked: &[String]) {
    let org = |key: &str| if locked.iter().any(|k| k == key) { "  (set by your organization)" } else { "" };
    match s.checkout_depth {
        0 => println!("history:           all commits{}", org("checkoutDepth")),
        n => println!("history:           the last {n} commits{}", org("checkoutDepth")),
    }
    let blobs = if s.blobs_on_demand { "when needed (installer files when their app is installed)" } else { "all at once" };
    println!("file contents:     {blobs}{}", org("blobsOnDemand"));
    let fallback = match s.fallback {
        furnish::settings::Fallback::Ask => "offer the previous version",
        furnish::settings::Fallback::Automatic => "install the previous version",
        furnish::settings::Fallback::Off => "nothing",
    };
    println!("when an install fails: {fallback}{}", org("fallback"));
    match s.fallback_max_age_days {
        0 => println!("previous versions: any age{}", org("fallbackMaxAgeDays")),
        d => println!("previous versions: at most {d} days older{}", org("fallbackMaxAgeDays")),
    }
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Register and start the service with the OS's service manager (as administrator)
    Install {
        /// macOS: let the account that owns Homebrew, an administrator, run sudo without a
        /// password, so the service can install casks that run installer packages
        #[arg(long)]
        homebrew_sudo: bool,
    },
    /// Stop and unregister the service (as administrator)
    Uninstall,
}

#[derive(Subcommand)]
enum OptionCommand {
    /// List the options of a collection or an app, their values and where those come from
    List { target: String },
    /// Set an option for a collection (all its apps) or for one app
    Set { target: String, name: String, value: String },
    /// Remove your value, so the collection's value or the default applies again
    Reset { target: String, name: String },
}

#[derive(Subcommand)]
enum CollectionCommand {
    /// Add a collection from a git URL
    Add {
        url: String,
        /// Defaults to the last part of the URL
        #[arg(long)]
        name: Option<String>,
        /// Answer one of the collection's options, e.g. --option role=developer (repeatable;
        /// required ones not given are asked for)
        #[arg(long = "option", value_name = "NAME=VALUE")]
        options: Vec<String>,
        /// HTTPS: the username for the git server (the password or token is asked for, or read
        /// with --password-stdin). Without it, git's credential helpers and prompt are used.
        #[arg(long)]
        username: Option<String>,
        /// HTTPS: read the password or access token from standard input (for scripts)
        #[arg(long, requires = "username")]
        password_stdin: bool,
    },
    /// List collections
    #[command(visible_alias = "ls")]
    List,
    /// Pull the latest version of one or all collections
    Update { name: Option<String> },
    /// Remove a collection (installed apps stay installed)
    Remove { name: String },
    /// Show or change settings for one collection (history kept, downloads, falling back)
    Settings {
        name: String,
        #[command(flatten)]
        settings: CollectionSettingsArgs,
        /// Use the general settings again (`furnish settings`)
        #[arg(long)]
        reset: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Command::Service { action: None }) {
        // The service keeps its collections, resources and records in the system directory.
        // SAFETY: still single-threaded.
        unsafe { std::env::set_var("FURNISH_DATA_DIR", catalog::system_dir()) };
        if let Err(err) = furnish::service::prepare() {
            eprintln!("✗ {err:#}");
            return ExitCode::FAILURE;
        }
    }
    if cli.verbose {
        engine::capture_logs(print_log_line);
    }
    let result = engine::init(&catalog::data_dir().join("resources")).and_then(|()| run(cli.command));
    // Reports this command queued (and older ones still waiting), if the organization takes them.
    furnish::reports::send();
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
    // Collections the admin provides are set up for every user.
    if !matches!(command, Command::Settings { .. } | Command::Reports | Command::Check { .. } | Command::Service { .. } | Command::Login { .. } | Command::Logout { .. }) {
        for problem in catalog::ensure_presets() {
            eprintln!("warning: {problem}");
        }
        // Collections preset by the policy or added by hand may still have open questions.
        if !matches!(command, Command::Option(_) | Command::Collection(CollectionCommand::Add { .. })) {
            for collection in catalog::collections()? {
                ask_unanswered(&collection)?;
            }
        }
    }
    match command {
        Command::Service { action: None } => furnish::service::serve(),
        Command::Service { action: Some(ServiceAction::Install { homebrew_sudo }) } => {
            furnish::service::install(homebrew_sudo)?;
            println!("✓ The furnish service is running. Enable it for users with the `service` section of the admin policy.");
            Ok(())
        }
        Command::Service { action: Some(ServiceAction::Uninstall) } => {
            furnish::service::uninstall()?;
            println!("✓ Removed the furnish service");
            Ok(())
        }
        Command::List => list(),
        Command::Login { endpoint } => {
            let origin = endpoint_origin(&endpoint)?;
            let description = furnish::endpoint::discover(&origin)?;
            furnish::endpoint::login(&origin, &description, |url| print_sign_in_url(&origin, url))?;
            println!("✓ Signed in to {}", description.name);
            for problem in catalog::ensure_presets() {
                eprintln!("warning: {problem}");
            }
            Ok(())
        }
        Command::Logout { endpoint } => {
            furnish::endpoint::logout(&endpoint_origin(&endpoint)?);
            println!("✓ Signed out");
            Ok(())
        }
        // Kits: names that are no app but a kit.
        Command::Install { app, dry_run, with, without, .. } if is_kit(&app) => {
            install_kit(&catalog::find_kit(&app)?, &with, &without, dry_run)
        }
        Command::Uninstall { app, dry_run, only } if is_kit(&app) => uninstall_kit(&catalog::find_kit(&app)?, &only, dry_run),
        Command::Uninstall { only, .. } if !only.is_empty() => anyhow::bail!("--only is for kits"),
        Command::Install { app, dry_run: true, with, without, .. } => {
            let app = catalog::find_app(&app)?;
            for dep in chosen_dependencies(&app, &with, &without)? {
                if !matches!(dep.status()?, Status::Installed | Status::NeedsUpdate | Status::Unavailable) {
                    println!("First, dependency {}:", dep.id);
                    print_plan(&dep, false)?;
                }
            }
            print_plan(&app, false)
        }
        Command::Uninstall { app, dry_run: true, .. } => print_plan(&catalog::find_app(&app)?, true),
        Command::Install { app, previous: true, .. } => {
            let app = catalog::find_app(&app)?;
            install_previous(&app, &app.previous_version()?)
        }
        Command::Install { app, current: true, with, without, .. } => {
            let app = catalog::find_app(&app)?;
            if app.held.is_none() {
                return install_with_dependencies(&app, &with, &without, &mut Vec::new());
            }
            println!("⟳ Installing the collection's current version of {}…", app.id);
            report_failure(&app, app.install_version(catalog::Which::Current, print_progress()))?;
            println!("✓ Installed {}", app.id);
            Ok(())
        }
        Command::Install { app, with, without, .. } => {
            install_with_dependencies(&catalog::find_app(&app)?, &with, &without, &mut Vec::new())
        }
        Command::Uninstall { app, .. } => uninstall_app(&catalog::find_app(&app)?),
        Command::Collection(cmd) => collection(cmd),
        Command::Upgrade { all } => upgrade(all),
        Command::Reinstall { app } => {
            let app = catalog::find_app(&app)?;
            // The current options may need other dependencies (e.g. Homebrew for a user install).
            let siblings = catalog::get(&app.collection)?.apps();
            for name in catalog::default_selection(&app.dependencies_for(&app.current_values())) {
                if let Some(dep) = siblings.iter().find(|a| a.id == name)
                    && !matches!(dep.status()?, Status::Installed | Status::NeedsUpdate | Status::Unavailable)
                {
                    println!("{} needs {} first", app.id, dep.id);
                    install_app(dep)?;
                }
            }
            println!("⟳ Reinstalling {} with its current options…", app.id);
            app.reinstall(print_progress())?;
            println!("✓ Reinstalled {}", app.id);
            Ok(())
        }
        Command::Option(cmd) => option(cmd),
        Command::Reports => reports(),
        Command::Check { dir } => check(&dir),
        Command::Settings { update_on_launch, update_interval, auto_update, no_auto_update, load_aware_updates, collections, reporting, report_hostname, fallback_language } => {
            if let Some(v) = fallback_language {
                furnish::settings::set("fallbackLanguage", v.into())?;
            }
            if let Some(v) = reporting {
                furnish::settings::set("reporting", v.into())?;
            }
            if let Some(v) = report_hostname {
                furnish::settings::set("reportHostname", v.into())?;
            }
            use furnish::settings::set;
            if let Some(v) = update_on_launch {
                set("updateOnLaunch", v.into())?;
            }
            if let Some(v) = update_interval {
                set("updateIntervalMinutes", v.into())?;
            }
            if let Some(v) = load_aware_updates {
                set("loadAwareUpdates", v.into())?;
            }
            for (key, value) in collections.changes() {
                set(key, value)?;
            }
            if !auto_update.is_empty() || !no_auto_update.is_empty() {
                let mut off = furnish::settings::load().no_auto_update;
                let key = |name: &str| catalog::find_app(name).map(|a| format!("{}/{}", a.collection, a.id));
                for name in &no_auto_update {
                    let key = key(name)?;
                    if !off.contains(&key) {
                        off.push(key);
                    }
                }
                for name in &auto_update {
                    let key = key(name)?;
                    off.retain(|k| k != &key);
                }
                set("noAutoUpdate", off.into())?;
            }
            let effective = furnish::settings::effective();
            for warning in &effective.warnings {
                eprintln!("warning: {warning}");
            }
            let s = effective.settings;
            let org = |key: &str| if effective.locked.iter().any(|k| k == key) { "  (set by your organization)" } else { "" };
            println!("update on launch:  {}{}", if s.update_on_launch { "yes" } else { "no" }, org("updateOnLaunch"));
            match s.update_interval_minutes {
                0 => println!("update interval:   never{}", org("updateIntervalMinutes")),
                m => println!("update interval:   every {m} minutes{}", org("updateIntervalMinutes")),
            }
            let l = &s.load_limits;
            if s.load_aware_updates {
                println!(
                    "when busy:         postpone automatic updates (5-minute average over CPU {}%, memory {}% or network {} KB/s){}",
                    l.cpu_percent, l.memory_percent, l.network_kbps, org("loadAwareUpdates")
                );
            } else {
                println!("when busy:         update anyway{}", org("loadAwareUpdates"));
            }
            match s.no_auto_update.as_slice() {
                [] => println!("app updates:       automatic for all apps{}", org("noAutoUpdate")),
                apps => println!("app updates:       automatic, except {}{}", apps.join(", "), org("noAutoUpdate")),
            }
            let level = match s.reporting {
                furnish::reports::Level::Off => "off",
                furnish::reports::Level::Errors => "failures and fallbacks",
                furnish::reports::Level::All => "everything, and the installed apps once a day",
            };
            println!("reports:           {level}{}{}", if s.report_hostname { ", with this computer's name" } else { "" }, org("reporting"));
            println!("fallback language: {} (for collection texts not in the app's language){}", s.fallback_language, org("fallbackLanguage"));
            let general: furnish::settings::CollectionSettings = serde_json::from_value(serde_json::to_value(&s)?)?;
            print_collection_settings(&general, &effective.locked);
            if !s.collections.is_empty() {
                let names: Vec<_> = s.collections.keys().map(String::as_str).collect();
                println!("Collections with their own settings (furnish collection settings <name>): {}", names.join(", "));
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
    let (wanted, locales) = (furnish::locale::wanted(), collection_locales());
    let none = furnish::locale::Locales::default();
    let texts = |collection: &str| locales.get(collection).unwrap_or(&none);
    let mut installed = std::collections::HashSet::new();
    let mut inventory = Vec::new();
    for app in &apps {
        let check = app.check();
        if let Ok(engine::Check { status: s @ (Status::Installed | Status::NeedsUpdate), .. }) = &check {
            installed.insert((app.collection.clone(), app.id.clone()));
            let status = if *s == Status::Installed { "installed" } else { "needsUpdate" };
            inventory.push(furnish::reports::Installed { app: app.key(), status });
        }
        let l = texts(&app.collection);
        let mut description = l.app_description(&wanted, &app.id, &app.description).to_string();
        let name = l.app_name(&wanted, &app.id, app.name.as_deref());
        if name != app.id {
            description = if description.is_empty() { name } else { format!("{name}: {description}") };
        }
        let declared = check.as_ref().ok().and_then(|c| c.declared.clone());
        let symbol = match check {
            Ok(c) => match c.status {
                Status::Installed => "✓",
                Status::NeedsUpdate => "↑",
                Status::NotInstalled => "·",
                Status::Unavailable => "–",
                Status::Blocked => {
                    let needs = c.reason.unwrap_or_default();
                    let deps_now = app.dependencies_for(&app.current_values());
                    let deps = catalog::default_selection(&deps_now);
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
        if let Some(commit) = &app.held {
            description = format!("{description} (held at an earlier version, {}: furnish install {} --current)", &commit[..7.min(commit.len())], app.id);
        }
        if app.options_differ() {
            description = format!("{description} (installed with other options: furnish reinstall {})", app.id);
        }
        if let Some(d) = &declared {
            description = format!("{description} (in {})", d.place());
        }
        if app.recommended() {
            description = format!("★ {description}");
        }
        let categories: Vec<&str> = app.categories.iter().map(|c| l.category(&wanted, c)).collect();
        rows.push(row([symbol, &app.id, &app.collection, &categories.join(", "), &description]));
    }
    print_table(&rows);
    if furnish::reports::inventory_due() {
        furnish::reports::inventory(inventory);
    }
    println!("\n✓ installed  ↑ update available  · not installed  – not available on this OS  ! needs a missing tool  ? unknown");

    let kits = catalog::all_kits()?;
    if !kits.is_empty() {
        println!("\nKits (install with `furnish install <kit>`, see --with/--without):\n");
        let mut rows = vec![row(["", "Kit", "Collection", "Items", "Description"])];
        for kit in &kits {
            let have = kit.items.iter().filter(|i| installed.contains(&(kit.collection.clone(), i.app.clone()))).count();
            let symbol = if have == kit.items.len() { "✓" } else if have > 0 { "◐" } else { "·" };
            let items = format!("{have} of {} installed", kit.items.len());
            let l = texts(&kit.collection);
            let mut description = l.kit_description(&wanted, &kit.id, &kit.description).to_string();
            let name = l.kit_name(&wanted, &kit.id, kit.name.as_deref());
            if name != kit.id {
                description = if description.is_empty() { name } else { format!("{name}: {description}") };
            }
            rows.push(row([symbol, &kit.id, &kit.collection, &items, &description]));
        }
        print_table(&rows);
    }
    Ok(())
}

/// Each collection's translations (see `locale`).
fn collection_locales() -> std::collections::HashMap<String, furnish::locale::Locales> {
    catalog::collections().unwrap_or_default().iter().map(|c| (c.name.clone(), c.locales())).collect()
}

fn collection(cmd: CollectionCommand) -> Result<()> {
    match cmd {
        // An organization's furnish address: sign in, then add all of its collections.
        CollectionCommand::Add { url, .. } if furnish::endpoint::origin(&url).is_some() => {
            let origin = furnish::endpoint::origin(&url).expect("checked");
            print_preview(&furnish::endpoint::preview(&origin)?);
            let added = catalog::add_endpoint(&origin, |url| print_sign_in_url(&origin, url))?;
            for c in &added {
                println!("✓ Added '{}' with {} app(s)", c.name, c.apps().len());
                ask_unanswered(c)?;
            }
            if added.is_empty() {
                println!("✓ Signed in; its collections were already added");
            }
        }
        CollectionCommand::Add { url, name, options, username, password_stdin } => {
            let name = name.unwrap_or_else(|| catalog::name_from_url(&url));
            let given = match (&username, password_stdin) {
                (Some(user), true) => {
                    let mut password = String::new();
                    std::io::stdin().read_line(&mut password)?;
                    Some((user.clone(), password.trim_end_matches(['\r', '\n']).to_string()))
                }
                (Some(_), false) => furnish::auth::ask_in_terminal(&url_with_user(&url, username.as_deref()))?,
                (None, _) => None,
            };
            let collection = match catalog::add(&url, &name, given.as_ref().map(|(u, p)| (u.as_str(), p.as_str()))) {
                // The server wants a login: ask like git does (its helpers, then the terminal).
                Err(err) if given.is_none() && err.is::<furnish::auth::CredentialsRequired>() => {
                    let (user, password) = furnish::auth::ask_in_terminal(&url)?.ok_or(err)?;
                    catalog::add(&url, &name, Some((&user, &password)))?
                }
                result => result?,
            };
            let apps = collection.apps().len();
            println!("✓ Added '{name}' at revision {} with {apps} app(s)", collection.revision()?);
            println!("⚠ Apps run with administrator rights. Only add collections from authors you trust.");
            let declared = collection.options()?;
            for option in &options {
                let (key, value) = option.split_once('=').with_context(|| format!("--option {option}: use NAME=VALUE"))?;
                let r = declared.iter().find(|r| r.name == key).with_context(|| format!("{name} has no option '{key}'"))?;
                furnish::options::set(&name, key, Some(furnish::options::parse(value, &r.declaration)?), &r.declaration)?;
            }
            ask_unanswered(&collection)?;
        }
        CollectionCommand::List => {
            let mut rows = vec![row(["Name", "Apps", "Revision", "URL", ""])];
            for c in catalog::collections()? {
                let (apps, rev, url) = (c.apps().len().to_string(), c.revision()?, c.url()?);
                let managed = if c.managed { "managed by your organization" } else { "" };
                rows.push(row([&c.name, &apps, &rev, &url, managed]));
            }
            print_table(&rows);
            if !furnish::settings::effective().users_can_add_collections {
                println!("\nYour organization manages the collections; adding others isn't allowed.");
            }
        }
        CollectionCommand::Update { name } => {
            let collections = match name {
                Some(name) => vec![catalog::get(&name)?],
                None => {
                    // All of them: the organizations' descriptions too, which may have changed.
                    for problem in furnish::endpoint::refresh() {
                        eprintln!("warning: {problem}");
                    }
                    catalog::collections()?
                }
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
        CollectionCommand::Settings { name, settings, reset } => {
            let collection = catalog::get(&name)?;
            if reset {
                for key in furnish::settings::PER_COLLECTION {
                    if !furnish::settings::for_collection(&name).locked.iter().any(|k| k == key) {
                        furnish::settings::set_for_collection(&name, key, serde_json::Value::Null)?;
                    }
                }
            }
            for (key, value) in settings.changes() {
                furnish::settings::set_for_collection(&name, key, value)?;
            }
            let effective = furnish::settings::for_collection(&name);
            print_collection_settings(&effective.settings, &effective.locked);
            if settings.checkout_depth.is_some() || settings.blobs_on_demand.is_some() {
                // Applied when git next talks to the server.
                collection.update()?;
            }
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

/// The key and resolved options of an option target: a collection, or an app (`app` or `collection/app`).
fn target(name: &str) -> Result<(String, Vec<furnish::options::Resolved>)> {
    use furnish::options;
    let collection = if name.contains('/') { None } else { catalog::get(name).ok() };
    match (collection, catalog::find_app(name)) {
        (Some(_), Ok(_)) => anyhow::bail!("'{name}' is a collection and an app; use collection/{name} for the app"),
        (Some(c), _) => Ok((c.name.clone(), options::resolve_collection(&c.name, &options::collection_declarations(&c.dir)?))),
        (None, Ok(a)) => Ok((a.key(), a.options())),
        (None, Err(_)) => anyhow::bail!("no collection or app named '{name}'"),
    }
}

/// The URL with a username, so git's prompt only asks for the password.
fn url_with_user(url: &str, user: Option<&str>) -> String {
    match (user, url.split_once("://")) {
        (Some(user), Some((scheme, rest))) => format!("{scheme}://{user}@{rest}"),
        _ => url.to_string(),
    }
}

/// Where people sign in, so a lookalike address (one using another organization's sign-in)
/// stands out, and the URL in case the browser doesn't open.
fn print_sign_in_url(origin: &str, url: &str) {
    let host = |u: &str| furnish::endpoint::url_host(u);
    println!("You sign in at {} for {}.", host(url), host(origin));
    println!("Signing in in your browser. If it doesn't open, go to:\n  {url}");
}

/// What adding a furnish address changes, before signing in.
fn print_preview(p: &furnish::endpoint::Preview) {
    println!("{} ({}): {}", p.name, furnish::endpoint::url_host(&p.origin), p.collections.join(", "));
    if !p.auto_install.is_empty() {
        println!("  Installs right away: {}", p.auto_install.join(", "));
    }
    match p.reporting.as_deref() {
        Some("all") => println!("  Turns on reports about its apps: installs, updates and removals, and the installed apps"),
        Some(_) => println!("  Turns on reports about its apps: failed installs, updates and removals"),
        None => {}
    }
    if p.branding {
        println!("  Shows furnish under its name and colors");
    }
}

fn endpoint_origin(input: &str) -> Result<String> {
    furnish::endpoint::origin(input).with_context(|| format!("'{input}' isn't a furnish address like furnish.myorg.tld"))
}

/// Ask for the collection's required options that have no value yet. Without a terminal, say how
/// to set them instead; the collection's apps can't be installed until then.
fn ask_unanswered(collection: &catalog::Collection) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    let mut unanswered = collection.unanswered();
    if unanswered.is_empty() {
        return Ok(());
    }
    let (wanted, locales) = (furnish::locale::wanted(), collection.locales());
    for r in &mut unanswered {
        locales.translate_option(&wanted, r);
    }
    if !std::io::stdin().is_terminal() {
        for r in &unanswered {
            eprintln!("⚠ {} asks: {}. Answer with: furnish option set {} {} <value>", collection.name, r.declaration.label.as_deref().unwrap_or(&r.name), collection.name, r.name);
        }
        return Ok(());
    }
    println!("{} asks {} question{} (Enter skips; its apps can't be installed until answered):", collection.name, unanswered.len(), if unanswered.len() > 1 { "s" } else { "" });
    for r in unanswered {
        println!("\n{}", r.declaration.label.as_deref().unwrap_or(&r.name));
        if let Some(d) = &r.declaration.description {
            println!("  {d}");
        }
        for (i, c) in r.choices.iter().enumerate() {
            let value = c.value.as_str().map(String::from).unwrap_or_else(|| c.value.to_string());
            println!("  {}) {}", i + 1, c.label.as_deref().unwrap_or(&value));
        }
        loop {
            print!("{}> ", if r.declaration.kind == "bool" && r.choices.is_empty() { "(yes/no) " } else { "" });
            std::io::stdout().flush()?;
            let mut line = String::new();
            if std::io::stdin().lock().read_line(&mut line)? == 0 {
                return Ok(());
            }
            let answer = line.trim();
            if answer.is_empty() {
                break;
            }
            // A number picks a choice; anything else is the value itself.
            let value = match answer.parse::<usize>().ok().and_then(|n| r.choices.get(n.wrapping_sub(1))) {
                Some(choice) => Ok(choice.value.clone()),
                None => furnish::options::parse(answer, &r.declaration),
            };
            match value.and_then(|v| furnish::options::set(&collection.name, &r.name, Some(v), &r.declaration)) {
                Ok(()) => break,
                Err(err) => println!("  {err:#}"),
            }
        }
    }
    Ok(())
}

fn option(cmd: OptionCommand) -> Result<()> {
    use furnish::options::{self, Source};
    let (key, resolved) = match &cmd {
        OptionCommand::List { target: t } | OptionCommand::Set { target: t, .. } | OptionCommand::Reset { target: t, .. } => target(t)?,
    };
    let find = |name: &str| {
        resolved.iter().find(|r| r.name == name).ok_or_else(|| anyhow::anyhow!("{key} has no option '{name}'"))
    };
    match cmd {
        OptionCommand::List { .. } => {
            if resolved.is_empty() {
                println!("{key} has no options.");
            }
            let mut rows = vec![row(["Option", "Value", "From", "Description"])];
            let locales = catalog::get(key.split('/').next().unwrap_or_default()).map(|c| c.locales()).unwrap_or_default();
            let wanted = furnish::locale::wanted();
            for r in &resolved {
                let mut r = r.clone();
                locales.translate_option(&wanted, &mut r);
                let value = r.value.as_ref().map_or("-".to_string(), |v| v.to_string());
                let from = match (r.source, r.locked) {
                    (Source::Policy, true) => "your organization (locked)",
                    (Source::Policy, false) => "your organization",
                    (Source::App, _) => "your value for this app",
                    (Source::Collection, _) => "your value for the collection",
                    (Source::Default, _) => "default",
                };
                let text = [r.declaration.label.clone(), r.declaration.description.clone()].into_iter().flatten().collect::<Vec<_>>().join(": ");
                rows.push(row([&r.name, &value, from, &text]));
            }
            print_table(&rows);
        }
        OptionCommand::Set { name, value, .. } => {
            let r = find(&name)?;
            options::set(&key, &name, Some(options::parse(&value, &r.declaration)?), &r.declaration)?;
            println!("✓ {name} = {value} for {key}");
        }
        OptionCommand::Reset { name, .. } => {
            let r = find(&name)?;
            options::set(&key, &name, None, &r.declaration)?;
            println!("✓ {name} for {key} follows the collection or default again");
        }
    }
    Ok(())
}

fn upgrade(all: bool) -> Result<()> {
    let settings = furnish::settings::load();
    let (mut updated, mut skipped) = (0, Vec::new());
    for app in catalog::all_apps()? {
        if app.status().ok() != Some(Status::NeedsUpdate) {
            continue;
        }
        let key = format!("{}/{}", app.collection, app.id);
        if !all && settings.no_auto_update.contains(&key) {
            skipped.push(key);
            continue;
        }
        furnish::reports::with_trigger("update", || install_app(&app))?;
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

/// `furnish check`: one line per app and kit, its findings below it, a summary; errors fail.
fn check(dir: &std::path::Path) -> Result<()> {
    use furnish::check::Severity;
    let items = furnish::check::collection(dir)?;
    let width = items.iter().map(|i| i.name.len()).max().unwrap_or(0);
    for item in &items {
        let worst = item.findings.iter().map(|f| f.severity).max();
        let symbol = match worst {
            Some(Severity::Error) => "✗",
            Some(Severity::Warning) => "!",
            None => "✓",
        };
        let systems: Vec<String> = item
            .systems
            .iter()
            .map(|(system, runs)| if runs.is_empty() { format!("{system}: –") } else { format!("{system}: {}", runs.join(", ")) })
            .collect();
        println!("{symbol} {:width$}  {}{}", item.name, item.kind, if systems.is_empty() { String::new() } else { format!("  {}", systems.join(" · ")) });
        for finding in &item.findings {
            let label = if finding.severity == Severity::Error { "error" } else { "warning" };
            println!("    {label}: {}", finding.message);
        }
    }
    let translations: Vec<_> = items.iter().filter_map(|i| Some((i.name.trim_start_matches("locales/").trim_end_matches(".yaml"), i.coverage.as_ref()?))).collect();
    if !translations.is_empty() {
        println!("\nTranslations (texts without one show in the fallback language or the collection's own):");
        for (lang, coverage) in translations {
            println!("  {lang}: {coverage}");
        }
    }
    let count = |s| items.iter().flat_map(|i| &i.findings).filter(|f| f.severity == s).count();
    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
    let apps = items.iter().filter(|i| i.kind == "app").count();
    let kits = items.iter().filter(|i| i.kind == "kit").count();
    let n = |count: usize, word: &str| format!("{count} {word}{}", if count == 1 { "" } else { "s" });
    println!("\n{}, {}: {}, {}", n(apps, "app"), n(kits, "kit"), n(errors, "error"), n(warnings, "warning"));
    if errors > 0 {
        anyhow::bail!("the collection has errors");
    }
    Ok(())
}

/// What furnish reports to the organization and what is waiting to be sent.
fn reports() -> Result<()> {
    let s = furnish::settings::effective();
    let level = serde_json::to_value(s.settings.reporting)?;
    let locked = if s.locked.iter().any(|k| k == "reporting") { " (set by your organization)" } else { "" };
    println!("Reports: {}{locked}", level.as_str().unwrap_or_default());
    match furnish::reports::destination_names().as_slice() {
        [] => println!("No furnish address of your organization takes reports; nothing is sent."),
        names => println!("Sent to: {}. Their proxy adds who you are signed in as.", names.join(", ")),
    }
    let queue = std::fs::read_to_string(furnish::reports::queue_path()).unwrap_or_default();
    let lines: Vec<_> = queue.lines().collect();
    println!("Waiting: {} ({})", lines.len(), furnish::reports::queue_path().display());
    for line in lines.iter().rev().take(20).rev() {
        println!("  {line}");
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
    // Dependencies whose condition holds for the values this install uses.
    let deps = app.dependencies_for(&app.installed_values());
    let names = catalog::choose(&deps, with, without, &format!("the dependencies of {}", app.id))?;
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
        if !matches!(dep.status()?, Status::Installed | Status::NeedsUpdate | Status::Unavailable) {
            println!("{} needs {} first", app.id, dep.id);
            install_with_dependencies(&dep, &[], &[], path)?;
        }
    }
    path.pop();
    let check = match app.check() {
        Ok(check) => check,
        Err(err) => return report_failure(app, Err(err)),
    };
    match check.status {
        Status::Installed => {
            match &check.declared {
                Some(d) => println!("✓ {} is already installed: it is in {}", app.id, d.place()),
                None => println!("✓ {} is already installed and up to date", app.id),
            }
            return Ok(());
        }
        Status::Unavailable => anyhow::bail!("{} is not available on this operating system", app.id),
        Status::Blocked => anyhow::bail!("{} needs {}", app.id, check.reason.unwrap_or_default()),
        Status::NotInstalled | Status::NeedsUpdate => {}
    }
    println!("⟳ Installing {}…", app.id);
    report_failure(app, app.install(print_progress()))?;
    println!("✓ Installed {}", app.id);
    Ok(())
}

/// After a failed install or update: with the fallback setting on, offer the previous version
/// (ask) or install it (automatic). Cancelled installs are no failure.
fn report_failure(app: &catalog::App, result: Result<()>) -> Result<()> {
    let Err(err) = result else { return Ok(()) };
    if err.is::<engine::Cancelled>() || app.held.is_some() {
        return Err(err);
    }
    let Ok(version) = app.previous_version() else { return Err(err) };
    match furnish::settings::for_collection(&app.collection).settings.fallback {
        furnish::settings::Fallback::Automatic => {
            eprintln!("✗ {err:#}");
            install_previous(app, &version)
        }
        _ => {
            eprintln!("The version of {} from {} may work: furnish install {} --previous", app.id, version.date, app.id);
            Err(err)
        }
    }
}

fn install_previous(app: &catalog::App, version: &catalog::Version) -> Result<()> {
    println!("⟳ Installing the version of {} from {}…", app.id, version.date);
    app.install_version(catalog::Which::Previous, print_progress())?;
    println!(
        "✓ Installed the version of {} from {}. furnish keeps it until the collection changes {}; furnish install {} --current tries the current version.",
        app.id, version.date, app.id, app.id
    );
    Ok(())
}

fn uninstall_app(app: &catalog::App) -> Result<()> {
    let check = app.check()?;
    if matches!(check.status, Status::NotInstalled | Status::Unavailable) {
        println!("✓ {} is not installed", app.id);
        return Ok(());
    }
    if let Some(d) = &check.declared {
        anyhow::bail!("{} is in {}: remove {} there", app.id, d.place(), d.package);
    }
    println!("⟳ Uninstalling {}…", app.id);
    app.uninstall(print_progress())?;
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
                if matches!(a.status(), Ok(Status::Installed | Status::NeedsUpdate)) {
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
        // Chosen by furnish, not by name: skip what furnish can't uninstall (named ones fail below).
        if only.is_empty()
            && let Some(what) = app.unremovable()?
        {
            println!("– {} (furnish can't uninstall it: {what} can't remove what it set up)", app.id);
            continue;
        }
        if only.is_empty()
            && let Some(d) = app.check()?.declared
        {
            println!("– {} (it is in {})", app.id, d.place());
            continue;
        }
        if dry_run { print_plan(app, true)?; } else { uninstall_app(app)?; }
    }
    Ok(())
}

fn print_plan(app: &catalog::App, uninstall: bool) -> Result<()> {
    use engine::Action;
    let steps = app.plan(uninstall)?;
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
