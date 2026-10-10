// Next.js configuration for the Stocksie frontend.
//
// The app lives under `app/` inside the Stocksie workspace, which contains
// multiple lockfiles (a root `yarn.lock` from the Anchor scaffold and this
// package's `pnpm-lock.yaml`). Next.js infers the monorepo root from the
// nearest lockfile, which can produce a misleading "inferred workspace root"
// warning and nondeterministic file tracing. `outputFileTracingRoot` pins it
// to the repo root explicitly so both the warning and standalone deployment
// tracing behave deterministically.

import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectDir = dirname(fileURLToPath(import.meta.url));

// Pin the workspace root for output file tracing (see file header) — but only
// inside the real Anchor workspace (the repo root holds Anchor.toml). On git
// hosts (Vercel) `app/` IS the uploaded root; pointing one level up makes
// Next look for `<root>/<root>/.next/routes-manifest.json` and the deploy
// dies after a successful build.
const workspaceRoot = existsSync(resolve(projectDir, "..", "Anchor.toml"))
  ? resolve(projectDir, "..")
  : undefined;

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,

  outputFileTracingRoot: workspaceRoot,

  webpack: (config, { isServer }) => {
    // `@solana/web3.js` / `@coral-xyz/borsh` reference a handful of Node-only
    // built-ins from code paths that never execute in the browser. Mark them
    // as empty so the browser bundle builds without pulling polyfills we
    // don't actually need at runtime.
    config.resolve.fallback = {
      ...config.resolve.fallback,
      fs: false,
      net: false,
      tls: false,
      dns: false,
      child_process: false,
      http2: false,
    };

    // No server-side bundling of browser-only Solana wallet code is required,
    // but keep the externals hook stable for future server entrypoints.
    if (isServer) {
      config.externals = config.externals ?? [];
    }

    return config;
  },

  experimental: {
    optimizePackageImports: ["@solana/web3.js", "@anchor-lang/core"],
  },
};

export default nextConfig;
