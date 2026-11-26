---
title: Install and manage apps
order: 4
group: Using Furnish
---
# Install and manage apps

## Find an app

Discover lists what your collections offer, starting with the apps they recommend for you. Installed and Updates narrow the list to installed apps and to apps with an update. The categories in the sidebar filter by topic. The search box matches names, descriptions and categories. It ignores accents and case, so "buro" finds "Büro", and it also finds texts in the fallback language.

A collection can ask a question when you add it, for example what you work on. furnish asks again at each start until you answer. Later closes the dialog; until you answer, the collection's apps show Answer first instead of Install.

## Sign in to your organization

If your organization has a furnish address, furnish signs you in with your organization's account in the browser. When that sign-in expires, a banner asks you to sign in again; until then furnish can't fetch new apps or updates from your organization.

When your organization adds a collection to its address, furnish adds it the next time it updates the collections. A collection you removed stays removed; to get it back, add the address again.

## An app's page

Click an app for its description, its source and what it sets up on this computer. Closed sections show details:

- Versions lists the app's versions from the collection's history, with the day of each. Install next to one installs that version (see [Earlier versions of an app](app-updates.md#earlier-versions-of-an-app)).
- **Steps** previews the commands an install or uninstall would run and whether they need administrator rights. For a package, it shows which manager and package name furnish chose on this computer and why. When a condition skips a step, its explanation appears above the original DSC expression. You can select and copy the expression.
- Activity log shows what the package manager printed during the last install.
- DSC configuration shows the app's file in the collection.

![Thunderbird's page with the preview of its install step.](images/app-detail.webp)

## Install, update, uninstall

While an app installs, its button shows the progress and a stop control. The first click stops after the current step; a second click stops the current step too. With Steps open, you see each step's start time, duration and result.

If a step needs administrator rights, your system asks for your password. Cancelling the password dialog cancels the install.

Some apps can't be uninstalled with furnish, for example a setting the collection doesn't say how to remove. Their page shows a note instead of the Uninstall button.

Apps you installed without furnish, for example with npm or the vendor's installer, can show as installed outside furnish. furnish doesn't install them a second time, update them or uninstall them; the app's page shows where the program is. Uninstall such an app with what installed it.

If an app belongs to your NixOS or Home Manager configuration, its page identifies the package to edit. See [NixOS deployment](nixos.md) or [Home Manager](home-manager.md) for configuration paths and install targets.

Installs run one at a time. **Activity** shows the current operation and the queue. Expand **Steps** to follow an app while it runs, or use ✕ to remove a waiting app from the queue.

![The Activity page: one install running, two waiting.](images/activity.webp)

## Options

Some apps can be installed in more than one way, for example for all users or only for you. An app's page shows its options; the Collections page shows the options that apply to all apps of a collection. Your organization may set some of them. If you change an option of an installed app, its page offers Reinstall with these options.

![The Collections page with the example collection's options open.](images/collections.webp)

## Language

Choose **Settings → Language** to override the system language. If a collection has no translation for that language, Furnish tries your **Fallback language**, then uses the author’s original text.

## Share an app or kit

Choose **Share** on the app or kit page. Select the action (install, update or uninstall), choose URL, HTML or Markdown, then select **Copy link**. The recipient reviews the action before Furnish applies it.

For example, `furnish://update?collection=work&app=editor` opens a confirmation for the editor update. The recipient must already have the named collection. If it is missing, they can add it separately where policy permits, then reopen the link.

See [Updates and recovery](app-updates.md) if an installation fails or an app needs an earlier version.
