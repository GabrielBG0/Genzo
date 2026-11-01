import type {
  Command,
  CommandResult,
  Event,
  Handshake,
  HandshakeResult,
  Query,
  QueryResult,
} from '$contract/genzo';

/**
 * What the app sees of the backend. Two implementations: the fake engine
 * (UI work and tests) and the Tauri transport (the product). Nothing else in
 * the app knows which one it is talking to.
 */
export interface Engine {
  readonly kind: 'fake' | 'tauri';
  handshake(hello: Handshake): Promise<HandshakeResult>;
  query(query: Query): Promise<QueryResult>;
  /**
   * Runs a command and resolves with its result; what it renders arrives as
   * events. Replies to commands sent in a row can resolve out of order: an
   * edit's `revision` says which is newest.
   */
  execute(command: Command): Promise<CommandResult>;
  /** Subscribes to engine events. Returns the unsubscribe function. */
  onEvent(listener: (event: Event) => void): () => void;
  /** The binary tile messages of a finished render generation. */
  fetchFrame(generation: number): Promise<ArrayBuffer>;
}
