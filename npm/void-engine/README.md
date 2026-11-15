# VOID Engine CLI

Install the npmjs.org package globally with:

```sh
npm install --global void-engine
```

Then sign in and request a build:

```sh
void login
void https://example.com --name "My App"
```

The installer downloads the matching client from the latest VOID Engine GitHub Release. Supported platforms are Apple Silicon macOS, x64 Linux, and x64 Windows. The install script is quiet on success and requires a network connection to GitHub Releases.

## GitHub Packages

This repository also publishes the scoped package `@nixhm55/void-engine` to GitHub Packages. Publishing is triggered manually from the **Publish VOID Engine CLI to GitHub Packages** workflow, or by publishing a GitHub Release tagged `void-engine-v<version>` where the version matches this package's `package.json`.

GitHub Packages requires authentication for npm installs. Configure the registry and sign in with a GitHub personal access token (classic) that has `read:packages`:

```sh
npm login --scope=@nixhm55 --auth-type=legacy --registry=https://npm.pkg.github.com
npm install --global @nixhm55/void-engine
```

The existing unscoped `void-engine` package on npmjs.org remains separate.
