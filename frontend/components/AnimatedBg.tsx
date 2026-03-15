import Image from "next/image";

interface Props {
  variant?: "landing" | "dashboard";
}

export default function AnimatedBg({ variant = "landing" }: Props) {
  const src =
    variant === "dashboard"
      ? "/bg/dashboard-background.webp"
      : "/bg/main-background.webp";

  return (
    <div className="anime-bg" aria-hidden="true">
      {/* Base image layer */}
      <Image
        src={src}
        alt=""
        fill
        priority
        quality={85}
        className="object-cover object-center"
        sizes="100vw"
      />

      {/* Dark overlay for text readability */}
      <div className="absolute inset-0 bg-bg-deep/40" />

      {/* CSS animated layers on top of image */}
      <div className="anime-bg-stars" />
      <div className="anime-bg-cloud anime-bg-cloud-1" />
      <div className="anime-bg-cloud anime-bg-cloud-2" />
      <div className="anime-bg-cloud anime-bg-cloud-3" />

      {/* Bottom vignette for smooth content blending */}
      <div className="absolute inset-x-0 bottom-0 h-64 bg-gradient-to-t from-bg-deep to-transparent" />
    </div>
  );
}
