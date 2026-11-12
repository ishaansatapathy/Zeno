"use client";

import { useInView } from "motion/react";
import { Fragment, useRef } from "react";

import { redacted } from "@/content/site";

import { Crosses, Label } from "./frame";

/**
 * A sentence with its best words blacked out, the way a document is redacted, here in paper
 * white on black. Scrolled into view, each bar wipes away in turn and the words underneath come
 * up in orange. The bar is the words' own background (cloned onto each line they wrap across),
 * and the words are the bar's colour until it goes, so nothing is ever positioned over text.
 */
export function Redacted() {
  const ref = useRef<HTMLParagraphElement>(null);
  const open = useInView(ref, { once: true, margin: "0px 0px -25% 0px" });

  return (
    <section aria-label={redacted.label} className="mx-auto max-w-page px-4 sm:px-6 lg:px-10">
      <div className="relative border border-line px-6 py-14 sm:px-12 lg:px-16 lg:py-24">
        <Crosses />
        <Label>{redacted.label}</Label>
        <p ref={ref} className="display mt-8 max-w-[62rem] text-[clamp(1.7rem,3.3vw,3.1rem)] leading-[1.18]">
          {redacted.before}{" "}
          {redacted.parts.map((part, i) => (
            <Fragment key={part.hidden}>
              <em
                className="italic transition-[background-size,color] duration-700 ease-[cubic-bezier(0.65,0,0.35,1)] [box-decoration-break:clone] [-webkit-box-decoration-break:clone]"
                style={{
                  backgroundImage: "linear-gradient(#f2efe9, #f2efe9)",
                  backgroundRepeat: "no-repeat",
                  backgroundPosition: "right center",
                  backgroundSize: open ? "0% 88%" : "100% 88%",
                  color: open ? "#ff5b14" : "#f2efe9",
                  transitionDelay: `${0.25 + i * 0.45}s`,
                }}
              >
                {part.hidden}
              </em>
              {part.after}{" "}
            </Fragment>
          ))}
        </p>
      </div>
    </section>
  );
}
