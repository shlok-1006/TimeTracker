import type { Metadata } from "next";
import "./globals.css";
import { Providers } from "./providers";
import { AppChrome } from "@/components/employee-shell";

export const metadata: Metadata = {
  title: "TimeTracker",
  description: "Employee portal for the TimeTracker platform",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="min-h-screen bg-background text-foreground antialiased">
        <Providers>
          <AppChrome>{children}</AppChrome>
        </Providers>
      </body>
    </html>
  );
}
