/**
 * Next.js config for the Tauri desktop frontend.
 *
 * Tauri loads the UI from a static export (`out/`) in production and from the
 * dev server (http://localhost:3000) during `tauri dev`. Static export requires
 * `output: 'export'` and unoptimized images (no Node server at runtime).
 */
/** @type {import('next').NextConfig} */
const nextConfig = {
  output: "export",
  reactStrictMode: true,
  images: { unoptimized: true },
  // Compile the shared workspace package from TypeScript source.
  transpilePackages: ["@timetracker/shared"],
  // Ensure asset paths work when loaded from the tauri:// custom protocol.
  trailingSlash: true,
};

export default nextConfig;
