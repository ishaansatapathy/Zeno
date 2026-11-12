import { Bench } from "@/components/site/Bench";
import { Faq } from "@/components/site/Faq";
import { Finale } from "@/components/site/Finale";
import { Flip } from "@/components/site/Flip";
import { Footer } from "@/components/site/Footer";
import { Hero } from "@/components/site/Hero";
import { Nav } from "@/components/site/Nav";
import { Pillars } from "@/components/site/Pillars";
import { Promises } from "@/components/site/Promises";
import { Providers } from "@/components/site/Providers";
import { Sides } from "@/components/site/Sides";

export default function Home() {
  return (
    <Providers>
      <Nav />
      <main id="main">
        <Hero />
        <Flip />
        <Sides />
        <Bench />
        <Pillars />
        <Promises />
        <Faq />
        <Finale />
      </main>
      <Footer />
    </Providers>
  );
}
