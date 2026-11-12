"use client";

import { useInView } from "motion/react";
import { Fragment, useRef } from "react";

import { redacted } from "@/content/site";

import { Container, Label } from "./frame";

/**
 * A sentence in gray with its best words redacted under solid bars. Scrolled into view, each bar
 * wipes away in turn and the words underneath come up in white. The bar is the words' own
 * background (cloned onto each line they wrap across), and the words are the bar's colour until
 * it goes, so nothing is ever laid over the text.
 */
export function Redacted() {
  const ref = useRef<HTMLParagraphElement>(null);
  const open = useInView(ref, { once: true, margin: "0px 0px -25% 0px" });

  return (
    <section aria-label={redacted.label} className="border-t border-line py-28 lg:py-40">
      <Container>
        <Label n="04">{redacted.label}</Label>
        <p ref={ref} className="display mt-8 max-w-[60rem] text-[clamp(1.8rem,3.6vw,3.3rem)] leading-[1.14] text-mute">
          {redacted.before}{" "}
          {redacted.parts.map((part, i) => (
            <Fragment key={part.hidden}>
              <span
                className="rounded-[3px] transition-[background-size,color] duration-700 ease-[cubic-bezier(0.65,0,0.35,1)] [box-decoration-break:clone] [-webkit-box-decoration-break:clone]"
                style={{
                  backgroundImage: "linear-gradient(#2b2d31, #2b2d31)",
                  backgroundRepeat: "no-repeat",
                  backgroundPosition: "right center",
                  backgroundSize: open ? "0% 84%" : "100% 84%",
                  color: open ? "#ededed" : "#2b2d31",
                  transitionDelay: `${0.25 + i * 0.4}s`,
                }}
              >
                {part.hidden}
              </span>
              {part.after}{" "}
            </Fragment>
          ))}
        </p>
      </Container>
    </section>
  );
}
