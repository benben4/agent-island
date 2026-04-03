import { useEffect, useRef } from 'react'
import type { MonitorNotification } from '../lib/types.js'

function tone(ctx: AudioContext, startAt: number, frequency: number, durationSec: number): void {
  const oscillator = ctx.createOscillator()
  const gain = ctx.createGain()
  oscillator.type = 'square'
  oscillator.frequency.value = frequency
  gain.gain.setValueAtTime(0.0001, startAt)
  gain.gain.exponentialRampToValueAtTime(0.08, startAt + 0.01)
  gain.gain.exponentialRampToValueAtTime(0.0001, startAt + durationSec)
  oscillator.connect(gain)
  gain.connect(ctx.destination)
  oscillator.start(startAt)
  oscillator.stop(startAt + durationSec)
}

export function useAudioNotifications(notifications: MonitorNotification[], enabled = false): void {
  const seenKeysRef = useRef<Set<string>>(new Set())
  const audioContextRef = useRef<AudioContext | null>(null)
  const initializedRef = useRef(false)

  useEffect(() => {
    const nextKeys = new Set<string>(notifications.map((notification) => notification.key))
    if (!initializedRef.current) {
      seenKeysRef.current = nextKeys
      initializedRef.current = true
      return
    }
    if (!enabled) {
      seenKeysRef.current = nextKeys
      return
    }
    if (notifications.length === 0) {
      seenKeysRef.current = nextKeys
      return
    }
    if (!audioContextRef.current) {
      audioContextRef.current = new window.AudioContext()
    }
    const ctx = audioContextRef.current
    for (const notification of notifications) {
      if (seenKeysRef.current.has(notification.key)) {
        continue
      }
      const startAt = ctx.currentTime + 0.02
      if (notification.kind === 'error') {
        tone(ctx, startAt, 220, 0.14)
        tone(ctx, startAt + 0.09, 180, 0.18)
      } else if (notification.kind === 'actionable') {
        tone(ctx, startAt, 520, 0.1)
        tone(ctx, startAt + 0.08, 660, 0.12)
      } else {
        tone(ctx, startAt, 660, 0.1)
        tone(ctx, startAt + 0.08, 880, 0.16)
      }
    }
    seenKeysRef.current = nextKeys
  }, [enabled, notifications])
}
