"use client";

import { m } from "motion/react";
import Image from "next/image";
import Link from "next/link";

import { join, links } from "@/content/site";

import { cards } from "./cards";
import { ArrowRight, Crown, Sparkle } from "./doodles";
import { DripButton } from "./DripButton";
import { Spray } from "./Spray";

export function Join() {
  return (
    <section aria-labelledby="join-title" className="relative mx-auto max-w-page px-4 pb-20 pt-16 sm:px-6 lg:px-8 lg:pb-28 lg:pt-24">
      <div className="grid items-center gap-16 lg:grid-cols-[minmax(0,1fr)_auto] lg:gap-10">
        <div>
          <h2 id="join-title" className="shout text-[clamp(3.2rem,7.6vw,7.8rem)]">
            <span className="block">{join.lead}</span>
            <span className="block">
              <Spray delay={0.4} underline>{join.sprayed}</Spray>
            </span>
          </h2>
          <p className="mt-10 max-w-[26rem] text-[0.98rem] leading-relaxed text-dust">{join.body}</p>
          <div className="mt-9 flex flex-wrap items-center gap-x-8 gap-y-6">
            <DripButton href={links.start} size="lg">
              {join.primary}
            </DripButton>
            <Link href={links.login} prefetch={false} className="caps group inline-flex items-center gap-2 text-[0.95rem] text-cream transition-colors hover:text-spray">
              {join.secondary}
              <ArrowRight className="size-4 transition-transform duration-300 group-hover:translate-x-1" />
            </Link>
          </div>
          <p className="caps mt-14 text-[0.72rem] tracking-[0.2em] text-dust">{join.foot}</p>
        </div>
        <m.div
          initial={{ opacity: 0, rotate: 22, y: 90 }}
          whileInView={{ opacity: 1, rotate: 7, y: 0 }}
          viewport={{ once: true, margin: "0px 0px -15% 0px" }}
          transition={{ type: "spring", stiffness: 70, damping: 13 }}
          className="relative mx-auto w-[min(70vw,19rem)] lg:mr-10 lg:w-[22rem]"
        >
          <Image src={cards[6]} alt={join.alt} sizes="(min-width: 1024px) 352px, 70vw" className="h-auto w-full drop-shadow-[0_34px_44px_rgba(0,0,0,0.7)]" />
          <Crown className="absolute -left-12 -top-8 w-14 -rotate-[18deg] text-spray" />
          <Sparkle className="absolute -right-8 bottom-[20%] w-9 text-cream" />
        </m.div>
      </div>
    </section>
  );
}
