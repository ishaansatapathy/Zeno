import { Deck } from "@/components/site/Deck";
import { Faq } from "@/components/site/Faq";
import { Footer } from "@/components/site/Footer";
import { Hero } from "@/components/site/Hero";
import { Join } from "@/components/site/Join";
import { Nav } from "@/components/site/Nav";
import { Process } from "@/components/site/Process";
import { Proof } from "@/components/site/Proof";
import { Protect } from "@/components/site/Protect";
import { Providers } from "@/components/site/Providers";
import { Statement } from "@/components/site/Statement";
import { Tape } from "@/components/site/Tape";

export default function Home() {
  return (
    <Providers>
      <Nav />
      <main id="main" className="overflow-x-clip">
        <Hero />
        <Statement />
        <Process />
        <Deck />
        <Tape />
        <Protect />
        <Proof />
        <Faq />
        <Join />
      </main>
      <Footer />
    </Providers>
  );
}
