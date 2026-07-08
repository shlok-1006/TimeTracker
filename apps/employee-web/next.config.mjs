/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  transpilePackages: ["@timetracker/shared"],
  // Served behind Nginx at a sub-path in production (NEXT_BASE_PATH=/employee).
  // Empty for local dev so the app stays at the root. Next.js prefixes routes
  // and _next assets with this automatically; raw fetch("/api/…") is unaffected.
  basePath: process.env.NEXT_BASE_PATH || "",
};

export default nextConfig;
