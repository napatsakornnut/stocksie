"use client";

// BarcodeScanner — camera-powered barcode reader for the `/scan` route
// (plan 006, Phase C).
//
// Wraps `html5-qrcode`'s `Html5Qrcode` class. The library is heavy and
// client-only (it touches `navigator.mediaDevices`), so this whole component
// is dynamically imported with `ssr: false` by `app/src/app/scan/page.tsx` —
// that keeps the scanner dep out of the `/` and `/shelf` First Load bundles.
//
// Lifecycle is parent-driven: the parent mounts <BarcodeScanner onScan={...} />
// when it wants a fresh scan, and unmounts (or changes `key`) to reset. We do
// NOT autostart the camera — a user gesture is required by some browsers, and
// on a desktop without a webcam the user should see an explicit "no camera"
// state rather than have the page fail silently on load. The component
// exposes a "Start camera" button; clicking it requests the back camera
// (`facingMode: "environment"`), begins scanning, and surfaces any failure
// (no camera / permission denied / insecure context) as a calm, specific
// message with a retry.
//
// On a successful decode we lock immediately (so the success callback can't
// fire twice for the same code), stop the camera to release the stream, then
// hand the code up via `onScan`. The parent decides what to do with it
// (lookup → known product card, or prefill the onboarding form).

import { useEffect, useId, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { Html5Qrcode, Html5QrcodeSupportedFormats } from "html5-qrcode";
import { Button } from "@/components/ui/Button";

/**
 * The decode formats to support. Restricted to retail-relevant formats
 * so ZXing doesn't burn the per-frame decode budget trying formats that
 * never appear on household products. Includes CODE_128 / CODE_39 / ITF
 * because non-grocery items (cosmetics, supplements, electronics) often
 * carry those instead of EAN/UPC — a too-narrow allow-list is a common
 * reason a clear barcode never decodes.
 */
const FORMATS_TO_SUPPORT: Html5QrcodeSupportedFormats[] = [
  Html5QrcodeSupportedFormats.QR_CODE,
  Html5QrcodeSupportedFormats.EAN_13,
  Html5QrcodeSupportedFormats.EAN_8,
  Html5QrcodeSupportedFormats.UPC_A,
  Html5QrcodeSupportedFormats.UPC_E,
  Html5QrcodeSupportedFormats.UPC_EAN_EXTENSION,
  Html5QrcodeSupportedFormats.CODE_128,
  Html5QrcodeSupportedFormats.CODE_39,
  Html5QrcodeSupportedFormats.ITF,
];

export type BarcodeScannerProps = {
  /**
   * Fired exactly once per successful scan with the decoded barcode string.
   * The component stops the camera and resets to "idle" before calling this,
   * so the parent can safely unmount or remount it without leaking a stream.
   */
  onScan: (code: string) => void;
};

/** Coarse state machine for the scanner's camera lifecycle. */
type Status =
  // Initial — no camera requested yet. The "Start camera" button is shown.
  | { kind: "idle" }
  // `start()` is in flight. Spinner; no buttons.
  | { kind: "starting" }
  // Camera is live and scanning. Stop button shown.
  | { kind: "scanning" }
  // A terminal failure (no camera, permission denied, insecure context).
  // Retry button shown.
  | { kind: "error"; message: string };

/**
 * Camera-only barcode scanner. See {@link BarcodeScannerProps} and the file
 * header for the lifecycle contract.
 */
export function BarcodeScanner({ onScan }: BarcodeScannerProps) {
  // Stable, unique DOM id for the div `html5-qrcode` mounts its <video> into.
  // `useId` keeps multiple scanners on one page from colliding (defensive —
  // the UI only ever shows one at a time).
  const reactId = useId();
  const elementId = `stocksie-scanner-${reactId.replace(
    /[^a-zA-Z0-9_-]/g,
    ""
  )}`;

  const [status, setStatus] = useState<Status>({ kind: "idle" });

  // The live `Html5Qrcode` instance, held in a ref so the cleanup effect and
  // the stop handler can both reach it without re-rendering on assignment.
  const scannerRef = useRef<Html5Qrcode | null>(null);
  // True once we've already handled a successful decode for this mount —
  // guards against `html5-qrcode` firing the success callback repeatedly.
  const scannedRef = useRef(false);

  // -------------------------------------------------------------------------
  // Cleanup on unmount: stop the camera and clear the DOM. `stop()` returns a
  // Promise (it has to await track.stop()); swallow errors here because we're
  // tearing down anyway and a rejected stop shouldn't surface as an unhandled
  // rejection. This runs whenever the parent unmounts us (e.g. after onScan).
  // -------------------------------------------------------------------------
  useEffect(() => {
    return () => {
      const scanner = scannerRef.current;
      scannerRef.current = null;
      if (!scanner) return;
      // The library throws if you stop a scanner that's already stopped or
      // not started — both are fine during teardown.
      scanner
        .stop()
        .then(() => scanner.clear())
        .catch(() => {
          /* already stopped / clearing — fine during unmount */
        });
    };
  }, []);

  // -------------------------------------------------------------------------
  // Start the camera. Wrapped in a function (not an effect) so it's tied to
  // the user's "Start camera" gesture. On any failure we surface a specific
  // message: insecure context, no camera, permission denied, or unknown.
  // -------------------------------------------------------------------------
  const handleStart = async () => {
    if (scannerRef.current || status.kind === "starting") return;
    scannedRef.current = false;

    // Flush the "starting" state to the DOM synchronously so the container
    // div below is display:block (with its min-height) BEFORE html5-qrcode
    // creates and plays the <video> during start(). Without this, the
    // container is display:none when the video element is created, and
    // Chrome acquires the camera stream (the green LED lights up) but
    // renders no frames — the feed area stays blank. flushSync is the
    // correct escape hatch here: we need the DOM updated and measured
    // before the imperative camera call on the very next line.
    flushSync(() => {
      setStatus({ kind: "starting" });
    });

    // Insecure context (HTTP, except localhost) → getUserMedia is unavailable.
    // Surfacing this distinctly saves the user a confusing "permission denied"
    // when the real problem is the page isn't HTTPS.
    if (typeof window !== "undefined" && !window.isSecureContext) {
      setStatus({
        kind: "error",
        message:
          "Camera access needs a secure (HTTPS) page. Open Stocksie over https:// or on localhost.",
      });
      return;
    }

    try {
      // Probe cameras before starting so we can give a specific "no camera"
      // message instead of letting `start()` reject with a generic error.
      const cameras = await Html5Qrcode.getCameras();
      if (!cameras || cameras.length === 0) {
        setStatus({
          kind: "error",
          message:
            "No camera found on this device. You can still type a barcode by hand below.",
        });
        return;
      }

      // Verbose logging is intentionally enabled: the library logs every
      // decode attempt and the per-frame ZXing/BarcodeDetector error to the
      // console. This is the only way to see WHY a clear barcode isn't
      // decoding — without it the scan loop fails silently. Open DevTools
      // → Console while scanning to see what the decoder reports. Once
      // scanning is reliably working this can be flipped back to false.
      const scanner = new Html5Qrcode(elementId, {
        verbose: true,
        formatsToSupport: FORMATS_TO_SUPPORT,
        // Force the ZXing JS decoder. The default (`true`) prefers Chrome's
        // native BarcodeDetector API, but that path has well-documented
        // reliability gaps for 1D barcodes (EAN/UPC/CODE_128) on macOS
        // Chrome/Safari — it silently returns no result for barcodes ZXing
        // decodes fine. ZXing is slower per frame but reliably detects the
        // formats in FORMATS_TO_SUPPORT.
        useBarCodeDetectorIfSupported: false,
      });
      scannerRef.current = scanner;

      await scanner.start(
        // Camera selection only. html5-qrcode validates that this object has
        // EXACTLY 1 key, and only accepts `facingMode` or `deviceId` — passing
        // width/height/resolution hints here throws
        // "'cameraIdOrConfig' object should have exactly 1 key". Resolution
        // constraints go in `videoConstraints` on the configuration object
        // below; when present, the library uses it directly and ignores this
        // argument's facingMode (so we duplicate facingMode into both spots).
        { facingMode: "environment" },
        {
          // fps 5 gives ZXing ~200ms per frame to decode instead of the
          // ~100ms budget at fps 10. The JS-only ZXing decoder (forced via
          // useBarCodeDetectorIfSupported: false above) needs the extra
          // headroom to avoid silently dropping frames.
          fps: 5,
          // 1D barcodes (EAN/UPC/CODE_128) have a canonical orientation —
          // they are not symmetric under horizontal flip like QR codes are.
          // The library's default behavior is to retry every failed decode
          // with the frame mirrored, which doubles the per-frame work for
          // zero benefit on 1D codes. Disabling flip halves decode time and
          // lets the next real frame arrive sooner.
          disableFlip: true,
          // Full MediaStreamConstraints — when present the library hands
          // this directly to getUserMedia (bypassing the facingMode selection
          // above). Request the back camera AND a high-resolution stream:
          // more pixels per barcode bar means ZXing can resolve the narrow
          // guard bars on dense EAN-13s that blur together on a 640x480 feed.
          videoConstraints: {
            facingMode: "environment",
            width: { ideal: 1280 },
            height: { ideal: 720 },
          },
          // Adaptive scan region. html5-qrcode silently disables the shaded
          // region (and falls back to full-frame scanning) when qrbox.height
          // exceeds the rendered video height, so this MUST scale to the
          // actual viewfinder. Wide-and-short matches 1D barcode geometry.
          qrbox: (viewfinderWidth, viewfinderHeight) => {
            // Width: fill most of the viewfinder, capped at 400 so the
            // region doesn't grow unboundedly on a large desktop window.
            const width = Math.max(50, Math.min(viewfinderWidth - 20, 400));
            // Height: 1D barcodes are horizontal stripes — a short region
            // matches their shape and avoids wasting decode budget on empty
            // vertical space. Cap at 150px and at 60% of viewfinder height,
            // whichever is smaller (the 60% guard guarantees the shaded
            // region renders — see the height-mismatch note above).
            const height = Math.max(
              50,
              Math.min(150, Math.floor(viewfinderHeight * 0.6))
            );
            return { width, height };
          },
        },
        (decodedText) => {
          // Lock against the library firing success repeatedly for the same
          // code in view. After the first decode, stop the camera and hand
          // the code up.
          if (scannedRef.current) return;
          scannedRef.current = true;
          console.info("[stocksie-scan] decoded:", decodedText);
          // Stop first (releases the stream promptly), THEN call onScan so
          // the parent's state update doesn't race the camera teardown.
          scanner
            .stop()
            .then(() => scanner.clear())
            .catch(() => {
              /* tear-down errors are non-fatal */
            })
            .finally(() => {
              scannerRef.current = null;
              setStatus({ kind: "idle" });
              onScan(decodedText);
            });
        },
        // Per-frame decode failure. Logging this (throttled) so we can see
        // what ZXing reports — previously this was silently swallowed, which
        // made it impossible to tell whether the decoder was running,
        // seeing garbage, or failing on an unsupported format. Once scanning
        // is reliably working this callback can go back to a no-op.
        createPerFrameErrorLogger()
      );
      setStatus({ kind: "scanning" });
      // Log the geometry so we can verify the qrbox / video math is sane.
      // If videoWidth/clientWidth is 0 or NaN, the drawImage source rect in
      // foreverScan becomes garbage and ZXing decodes an empty canvas —
      // this log surfaces that without needing a separate repro.
      logScanGeometry(elementId);
    } catch (err) {
      // Most likely permission denied or hardware error. Map a few known
      // shapes to friendlier copy; otherwise surface the raw message.
      scannerRef.current = null;
      const message = describeStartError(err);
      setStatus({ kind: "error", message });
    }
  };

  // -------------------------------------------------------------------------
  // Manual stop (the Stop button). Mirrors the cleanup path: stop, clear,
  // null the ref, reset state.
  // -------------------------------------------------------------------------
  const handleStop = async () => {
    const scanner = scannerRef.current;
    if (!scanner) {
      setStatus({ kind: "idle" });
      return;
    }
    try {
      await scanner.stop();
      scanner.clear();
    } catch {
      /* already stopped */
    } finally {
      scannerRef.current = null;
      setStatus({ kind: "idle" });
    }
  };

  return (
    <div className="flex flex-col gap-3">
      {/* The mount point for html5-qrcode's <video>. Visible (block) during
          both `starting` and `scanning` so html5-qrcode sees a non-zero-sized
          container when it creates and plays the video — see the flushSync
          call in handleStart for why this matters. The min-height keeps the
          empty placeholder from collapsing to zero before the video stream
          arrives. Hidden in idle/error so it doesn't take layout space. */}
      <div
        id={elementId}
        className={
          "overflow-hidden rounded-lg border border-stone-200 dark:border-slate-800 bg-white dark:bg-slate-950 min-h-[240px] " +
          (status.kind === "scanning" || status.kind === "starting"
            ? "block"
            : "hidden")
        }
        aria-hidden={status.kind !== "scanning"}
      />

      <ScannerControls
        status={status}
        onStart={handleStart}
        onStop={handleStop}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Controls — varies by status
// ---------------------------------------------------------------------------

type ScannerControlsProps = {
  status: Status;
  onStart: () => void;
  onStop: () => void;
};

function ScannerControls({ status, onStart, onStop }: ScannerControlsProps) {
  if (status.kind === "idle") {
    return (
      <Button onClick={onStart}>
        <span aria-hidden="true">📷</span>
        Start camera
      </Button>
    );
  }

  if (status.kind === "starting") {
    return (
      <Button loading disabled>
        Starting camera…
      </Button>
    );
  }

  if (status.kind === "scanning") {
    return (
      <div className="flex flex-col gap-2">
        <p className="text-xs text-stone-500 dark:text-slate-400">
          Point at a product barcode. It scans automatically.
        </p>
        <Button variant="ghost" onClick={onStop}>
          Stop camera
        </Button>
      </div>
    );
  }

  // error
  return (
    <div className="flex flex-col gap-2 rounded-lg border border-amber-500/30 bg-amber-50 dark:bg-amber-500/5 px-4 py-3">
      <p className="text-xs leading-relaxed text-amber-700/90 dark:text-amber-200/90">
        {status.message}
      </p>
      <div className="flex items-center gap-2">
        <Button variant="secondary" size="sm" onClick={onStart}>
          Try again
        </Button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/**
 * Translate an `html5-qrcode` start failure into a friendly message. The
 * library (and the underlying getUserMedia) throws a variety of shapes; we
 * match on substrings of the message rather than relying on a stable type.
 */
function describeStartError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const lower = raw.toLowerCase();

  if (
    lower.includes("permission") ||
    lower.includes("notallowederror") ||
    lower.includes("denied")
  ) {
    return "Camera permission was blocked. Allow camera access in your browser, then try again — or type the barcode by hand below.";
  }
  if (
    lower.includes("notfound") ||
    lower.includes("notreadableerror") ||
    lower.includes("trackstart")
  ) {
    return "Couldn't open the camera (it may be in use by another app). Close other apps using the camera, then try again — or type the barcode by hand below.";
  }
  if (lower.includes("notsupported") || lower.includes("mediaDevices")) {
    return "This browser doesn't expose a camera API. Type the barcode by hand below.";
  }
  return `Couldn't start the camera: ${raw}. You can still type a barcode by hand below.`;
}

// ---------------------------------------------------------------------------
// Debug helpers — surface what ZXing is actually seeing / reporting.
// These log to the console so a developer watching DevTools while a scan
// fails can tell *why* it failed (empty canvas, wrong region, unsupported
// format, focus blur, etc.) instead of guessing. All of this is safe to
// delete once scanning is reliably working in production.
// ---------------------------------------------------------------------------

/**
 * Returns a per-frame error callback that throttles its logging to one line
 * per second. ZXing fires this on every frame without a code (~5-10×/s) so
 * unthrottled logging floods the console. The throttled version still shows
 * the *type* of decode failure, which is the missing diagnostic.
 */
function createPerFrameErrorLogger(): (msg: string) => void {
  let lastLogAt = 0;
  let lastMsg = "";
  return (msg: string) => {
    const now = Date.now();
    if (now - lastLogAt < 1000 && msg === lastMsg) return;
    lastLogAt = now;
    lastMsg = msg;
    console.info("[stocksie-scan] per-frame decode failed:", msg);
  };
}

/**
 * Log the video element's geometry + the dimensions of the decode canvas.
 * If `videoWidth`/`clientWidth` is 0 or NaN, the library's `foreverScan`
 * computes invalid `drawImage` source rectangles and ZXing ends up decoding
 * a blank canvas — which presents exactly as "the perf log shows decodes
 * happening but no barcode is ever found."
 */
function logScanGeometry(elementId: string): void {
  // The geometry isn't available until the video has loaded metadata, so
  // defer one tick + give the stream time to spin up.
  setTimeout(() => {
    const container = document.getElementById(elementId);
    if (!container) {
      console.warn("[stocksie-scan] container not found:", elementId);
      return;
    }
    const video = container.querySelector("video");
    const canvas = container.querySelector("canvas");
    console.info("[stocksie-scan] geometry:", {
      container: {
        clientWidth: container.clientWidth,
        clientHeight: container.clientHeight,
      },
      video: video
        ? {
            videoWidth: video.videoWidth,
            videoHeight: video.videoHeight,
            clientWidth: video.clientWidth,
            clientHeight: video.clientHeight,
            readyState: video.readyState,
            currentTime: video.currentTime,
          }
        : null,
      canvas: canvas ? { width: canvas.width, height: canvas.height } : null,
    });
  }, 500);
}
