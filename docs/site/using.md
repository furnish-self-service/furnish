---
title: Install and manage apps
order: 4
group: Using Furnish
---
# Install and manage apps

## Find an app

Discover lists what your collections offer, starting with the apps they recommend for you. Installed and Updates narrow the list to installed apps and to apps with an update. The categories in the sidebar filter by topic. The search box matches names, descriptions and categories. It ignores accents and case, so "buro" finds "Büro", and it also finds texts in the fallback language.

A collection can ask a question when you add it, for example what you work on. furnish asks again at each start until you answer. Later closes the dialog; until you answer, the collection's apps show Answer first instead of Install.

![The question dialog: "What do you work on?" with a list of roles.](images/questions.webp)

## Sign in to your organization

If your organization has a furnish address, furnish signs you in with your organization's account in the browser. When that sign-in expires, a banner asks you to sign in again; until then furnish can't fetch new apps or updates from your organization.

When your organization adds a collection to its address, furnish adds it the next time it updates the collections. A collection you removed stays removed; to get it back, add the address again.

![The banner asking to sign in to Acme.](images/sign-in.webp)

## An app's page

Click an app for its description, its source and what it sets up on this computer. Closed sections show details:

- Versions lists the app's versions from the collection's history, with the day of each. Install next to one installs that version (see [Earlier versions of an app](app-updates.md#earlier-versions-of-an-app)).
- **Steps** previews the commands an install or uninstall would run and whether they need administrator rights. For a package, it shows which manager and package name furnish chose on this computer and why. When a condition skips a step, its explanation appears above the original DSC expression. You can select and copy the expression.
- Activity log shows what the package manager printed during the last install.
- DSC configuration shows the app's file in the collection.

![Thunderbird's page with the preview of its install step.](images/app-detail.webp)

## Install, update, uninstall

While an app installs, its button shows the progress and a stop control. The first click stops after the current step; a second click stops the current step too. With Steps open, you see each step's start time, duration and result.

![An install in progress: a step skipped on this system, then the running step.](images/install-progress.webp)

If a step needs administrator rights, your system asks for your password. Cancelling the password dialog cancels the install.

Some apps can't be uninstalled with furnish, for example a setting the collection doesn't say how to remove. Their page shows a note instead of the Uninstall button.

Apps you installed without furnish, for example with npm or the vendor's installer, can show as installed outside furnish. furnish doesn't install them a second time, update them or uninstall them; the app's page shows where the program is. Uninstall such an app with what installed it.

On NixOS, apps from your NixOS or Home Manager configuration show as installed via that configuration. furnish doesn't install, update or uninstall them. To remove one, delete its package from your configuration; the app's page shows which one. Some apps go into your NixOS configuration: furnish adds them there, rebuilds the system and commits `flake.lock` for you. Your administrator sets this up. If you use Home Manager with furnish's Home Manager module, these apps go into your Home Manager configuration instead, and furnish activates the new generation for you. The Settings page shows the lines your configuration needs for this, and where apps go. You can choose the NixOS configuration or Home Manager there, or with `furnish settings --nixos-install-into system` or `home`.

If your NixOS configuration sets `services.furnish.nix.configuration`, furnish uses that folder. Otherwise it looks in the usual places, such as `/etc/nixos` or nh's `NH_FLAKE`. The NixOS section of the Settings page shows what it found and how. If it found nothing, or the wrong folder, enter your configuration's folder there, or pick one of the suggested folders. You can also set the name in the flake if it isn't your computer's name. "Find it automatically" goes back to searching. In the terminal: `furnish settings --nixos-configuration ~/nixos-config`, `--nixos-flake-attribute <name>` and `--nixos-reset`.

Installs run one at a time. **Activity** shows the current operation and the queue. Expand **Steps** to follow an app while it runs, or use ✕ to remove a waiting app from the queue.

![The Activity page: one install running, two waiting.](images/activity.webp)

## Options

Some apps can be installed in more than one way, for example for all users or only for you. An app's page shows its options; the Collections page shows the options that apply to all apps of a collection. Your organization may set some of them. If you change an option of an installed app, its page offers Reinstall with these options.

![The Collections page with the example collection's options open.](images/collections.webp)

## Language

furnish is available in English, German, French, Spanish, Italian, Polish, Portuguese and Dutch. It follows the system language; Language on the Settings page sets one of them instead. For any other system language it uses English.

Collections can translate their app names, descriptions, categories and questions. furnish shows them in its own language if the collection has that translation. Otherwise it uses the fallback language, which you choose under Fallback language on the Settings page. It is English unless your organization sets another. Texts that exist in neither language appear as the collection wrote them.

![The Discover page in German.](images/discover-de.webp)

## Share an app or kit

Choose **Share** on the app or kit page. Select the action (install, update or uninstall), choose URL, HTML or Markdown, then select **Copy link**. The recipient reviews the action before Furnish applies it.

For example, `furnish://update?collection=work&app=editor` opens a confirmation for the editor update. The recipient must already have the named collection. If it is missing, they can add it separately where policy permits, then reopen the link.

See [Updates and recovery](app-updates.md) if an installation fails or an app needs an earlier version.
