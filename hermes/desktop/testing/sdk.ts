export const shared = { owner: { connectionId: 'local-A', profile: 'work' }, session: 'runtime-A', submissions: [] as unknown[], keyReads: [] as any[], key: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', routes: [{ connectionId: 'local-A', profile: 'work', targetProfile: 'work', mode: 'local' }], failSubmit: false, failLaunch: false, launchWait: null as Promise<void> | null, runtime: { instance_id: 'runtime-instance-A', profile_id: 'profile-A', authority_epoch: 1, capabilities: ['durable-admission-v1'] } };
export const host = {
  state: { focusedSessionOwner: { get: () => shared.owner }, focusedSessionId: { get: () => shared.session } },
  profileRoutes: async () => shared.routes,
  retainProfile: async () => () => {},
  requestProfile: async (route: unknown, method: string, params: unknown) => {
    if (method === 'runtime.describe') return shared.runtime;
    if (method === 'session.resume') return { session_id: (params as { session_id: string }).session_id, authority_epoch: shared.runtime.authority_epoch };
    if (method === 'shell.exec' && (params as { command: string }).command.includes('cat "$apparent_bridge_dir/')) {
      shared.keyReads.push({ route: { ...route as object }, params: { ...params as object } });
      return { code: 0, stdout: shared.key, stderr: '' };
    }
    shared.submissions.push({ route: { ...route as object }, method, params: { ...params as object } });
    if (method === 'shell.exec') await shared.launchWait;
    if (method === 'shell.exec' ? shared.failLaunch : shared.failSubmit) throw new Error('Lost reply');
    return method === 'shell.exec' ? { code: 0, stdout: '', stderr: '' } : { admission_id: 'accepted' };
  },
  notify: () => {},
};
export const COMPOSER_AREAS = { actions: 'composer.actions' };
export const PALETTE_AREA = 'palette';
export const Button = () => {};
export const Codicon = () => {};
export const jsx = (type: unknown, props: unknown) => ({ type, props });
export const atom = (initial: unknown) => { let value = initial; return { get: () => value, set: (next: unknown) => { value = next; } }; };
export const useValue = (value: { get(): unknown }) => value.get();
