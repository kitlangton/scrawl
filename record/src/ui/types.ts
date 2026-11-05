export type PocketView = {tone:'info'|'warning'|'failure';title:string;description:string;actionIds:string[]};
export type PillView = {
  revision:number; phase:'hidden'|'selecting'|'recording'|'processing'|'completed';
  clock:string; label:string; warning:string|null;notice:string|null; reducedMotion:boolean; draggable:boolean;inputLevel:number|null;
  /** Visible status for selecting-phase hosts that have no target-picker controls. */
  status?:string;
  pocket?:PocketView|null;
  noteCount:number;
  actions:{id:string;label:string;enabled:boolean;selected:boolean;primary:boolean}[];
};
/** Existing Desktop host capability consumed by the pill presentation. */
export type PillHost = {
  read:()=>Promise<PillView|null>;
  layout:(revision:number)=>Promise<void>;
  manualDrag:boolean;
  beginDrag:(revision:number)=>Promise<void>;
  beginPaste:(revision:number)=>Promise<void>;
  cancelPaste:()=>Promise<void>;
  act:(revision:number,action:string,drop?:boolean)=>Promise<void>;
  subscribe:(callback:(state:PillView)=>void)=>()=>void;
};
