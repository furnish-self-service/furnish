---
title: Get started
order: 1
group: Using Furnish
---
# Get started

Furnish lets you install apps from a catalog your organization maintains. Each catalog is a Git repository called a **collection**.

1. [Install Furnish](install.md) for your operating system.
2. Open **Collections** and add your organization's Furnish address or a collection's Git URL.
3. Open **Discover**, choose an app, and expand **Steps** to preview the changes. Select **Install** when ready.

## Add a collection

If your organization gave you a Furnish address, enter it in **Collections** and sign in through your browser. Answer any setup questions to see recommendations for your role.

To try the public example catalog, enter its Git URL in Collections or run:

```sh
furnish collection add https://github.com/furnish-self-service/self-service-example.git
```

## Next

- [Install and manage apps](using.md) or [use the CLI](cli.md).
- [Write a collection](collections.md) to publish apps for your team.
- [Administer Furnish](admins.md) to deploy it across managed devices.
