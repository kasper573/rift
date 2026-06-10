# rift

An online RPG built in rust.

I'm doing this project for fun and to teach myself more about multiplayer game development and web development infrastructure.

## Development

### Initial setup (only required once)

- Install [Docker](https://www.docker.com/)

### Before each development session

- Open the devcontainer in vscode or via devcontainer CLI.
- Build the artifacts and start the stack:

  ```sh
  cargo build --release -p website -p server && cargo wasm
  docker compose -f docker/docker-compose.yaml --profile test up -d --build --wait
  ```

- Visit `https://rift.localhost` in your browser

## Production deployment

This repository comes with a github actions workflow that performs automatic
deployments whenever the main branch receives updates. It's a simple deploy
script designed to deploy to a single remote machine. It logs in to your remote
machine via ssh and updates or initializes the docker stack utilizing the same
docker compose file as in development but with production environment variables
provided via github action variables and secrets.

Review the workflow to see which variables and secrets you need to provide.
