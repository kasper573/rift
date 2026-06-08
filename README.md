# rift

An online rpg built in rust.

I'm doing this project for fun and to teach myself more about multiplayer game development and web development infrastructure.

## Development

### Initial setup (only required once)

- Install [Docker](https://www.docker.com/)

### Before each development session

- Open the devcontainer in vscode or via devcontainer CLI.
- Run `cargo x dev`
- Visit `https://mp.localhost` in your browser

## Production deployment

This repository comes with a github actions workflow that performs automatic
deployments whenever the main branch receives updates. It's a simple deploy
script designed to deploy to a single remote machine. It logs in to your remote
machine via ssh and updates or initializes the docker stack utilizing the same
docker compose file as in development but with production environment variables
provided via github action variables and secrets.

Review the workflow to see which variables and secrets you need to provide.

## Monorepo convention

The crates in this repo is grouped into layers.

Lower levels may not depend on higher levels.

These are the layers, ordered highest to lowest:

### app

Deployable executables. Most business logic exist here.

May depend on other apps, but it's preferable to do so via protocol (ie. http requests) rather than direct dependency on code.

May depend on lib crates.

### lib

Generic and low level systems. May not depend on any app crate. May depend on other lib crates.
