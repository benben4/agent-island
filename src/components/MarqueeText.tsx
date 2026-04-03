interface MarqueeTextProps {
  text: string
  className?: string
  running?: boolean
}

export function MarqueeText({ text, className, running = false }: MarqueeTextProps) {
  const shouldAnimate = running && text.trim().length > 12

  return (
    <span className={['marquee-text', shouldAnimate ? 'running' : '', className ?? ''].filter(Boolean).join(' ')} title={text}>
      <span className="marquee-track">
        <span className="marquee-copy">{text}</span>
        {shouldAnimate ? <span className="marquee-gap" aria-hidden="true">   </span> : null}
        {shouldAnimate ? <span className="marquee-copy" aria-hidden="true">{text}</span> : null}
      </span>
    </span>
  )
}
