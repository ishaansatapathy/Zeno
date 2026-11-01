import "./globals.css";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Zeno",
  description: "Secure video and live-class platform for Indian educators",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
