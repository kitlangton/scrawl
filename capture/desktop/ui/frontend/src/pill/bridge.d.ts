import type {PillHost} from '@apparent/record/ui';

declare global {
  interface Window {
    apparentPill?: PillHost;
  }
}

export {};
