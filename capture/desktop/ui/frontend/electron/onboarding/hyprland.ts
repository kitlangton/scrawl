import {execFile} from 'node:child_process';

// OS mechanics only. Match this host's single owned window; never the active
// window, an app-wide class rule, or a user's persistent compositor config.
export function borderCommand(pid: number, title: string) {
  if (!Number.isSafeInteger(pid) || pid <= 0 || title !== `Apparent onboarding ${pid}`) throw Error('Invalid onboarding window identity');
  return `local m = hl.get_windows({ title = '${title}' }); assert(#m == 1 and m[1].pid == ${pid} and m[1].xwayland, 'Onboarding window not mapped'); hl.dispatch(hl.dsp.window.set_prop({ window = m[1], prop = 'border_size', value = '0' }))`;
}

export async function removeCompositorBorder(pid: number, title: string) {
  if (process.platform !== 'linux' || !process.env.HYPRLAND_INSTANCE_SIGNATURE) return;
  const script = borderCommand(pid, title);
  await new Promise<void>((resolve, reject) => execFile('hyprctl', ['eval', script],
    {timeout: 750, maxBuffer: 8192}, (error, stdout) => {
      if (error || stdout.trim() !== 'ok') reject(Error('Could not remove onboarding compositor border'));
      else resolve();
    }));
}
