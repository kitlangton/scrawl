/** Internal capability used only after negotiated recovery-role startup.
 * Closing pending local RPCs is separate from native process lifetime.
 */
export const releaseLocalRequests: unique symbol = Symbol("release recovery RPC ownership");
