import { Close } from "@/components/site/Close";
import { Hero } from "@/components/site/hero/Hero";
import { Live } from "@/components/site/live/Live";
import { Money } from "@/components/site/money/Money";
import { Pipeline } from "@/components/site/pipeline/Pipeline";
import { Students } from "@/components/site/students/Students";
import { ZenoRail } from "@/components/site/ZenoRail";

/**
 * The home page, as one line from top to bottom: what Zeno is, how a lesson travels, where the
 * money goes, teaching live, the student's side, and the one thing to do next.
 */
export default function Home() {
  return (
    <>
      <ZenoRail />
      <Hero />
      <Pipeline />
      <Money />
      <Live />
      <Students />
      <Close />
    </>
  );
}
