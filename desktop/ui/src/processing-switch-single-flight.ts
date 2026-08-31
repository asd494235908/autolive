export type ProcessingSwitchSingleFlight = {
  busy: boolean;
};

export function createProcessingSwitchSingleFlight(): ProcessingSwitchSingleFlight {
  return { busy: false };
}

export async function runProcessingSwitchSingleFlight<T>(
  gate: ProcessingSwitchSingleFlight,
  setBusy: (busy: boolean) => void,
  operation: () => Promise<T>,
): Promise<T | undefined> {
  if (gate.busy) return undefined;
  gate.busy = true;
  setBusy(true);
  try {
    return await operation();
  } finally {
    gate.busy = false;
    setBusy(false);
  }
}
