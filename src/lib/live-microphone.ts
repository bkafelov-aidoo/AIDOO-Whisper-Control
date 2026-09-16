export interface MicrophoneAcquireOptions {
  attemptTimeoutMs: number;
  retryDelayMs: number;
  timeoutMessage?: string;
}

interface StoppableMediaStream {
  getTracks(): Array<{ stop(): void }>;
}

class MediaRequestTimeoutError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MediaRequestTimeoutError";
  }
}

export function mediaRequestWithTimeout<T extends StoppableMediaStream>(
  request: Promise<T>,
  timeoutMs: number,
  message: string,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let finished = false;
    const timeout = setTimeout(() => {
      if (finished) return;
      finished = true;
      reject(new MediaRequestTimeoutError(message));
    }, timeoutMs);

    request.then(
      (stream) => {
        if (finished) {
          stream.getTracks().forEach((track) => track.stop());
          return;
        }
        finished = true;
        clearTimeout(timeout);
        resolve(stream);
      },
      (reason) => {
        if (finished) return;
        finished = true;
        clearTimeout(timeout);
        reject(reason);
      },
    );
  });
}

export async function acquireMicrophone<T extends StoppableMediaStream>(
  request: (constraints: MediaStreamConstraints) => Promise<T>,
  constraints: MediaStreamConstraints,
  options: MicrophoneAcquireOptions,
): Promise<T> {
  const message = options.timeoutMessage ?? "Микрофонът не отговори навреме.";
  try {
    return await mediaRequestWithTimeout(request(constraints), options.attemptTimeoutMs, message);
  } catch (reason) {
    if (!(reason instanceof MediaRequestTimeoutError)) throw reason;
  }

  if (options.retryDelayMs > 0) {
    await new Promise<void>((resolve) => setTimeout(resolve, options.retryDelayMs));
  }
  return mediaRequestWithTimeout(request(constraints), options.attemptTimeoutMs, message);
}
