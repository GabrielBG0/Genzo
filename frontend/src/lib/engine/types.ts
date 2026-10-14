import type {
  Command,
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
  /** Runs a command. Its results arrive as events. */
  execute(command: Command): Promise<void>;
  /** Subscribes to engine events. Returns the unsubscribe function. */
  onEvent(listener: (event: Event) => void): () => void;
  /** The binary tile messages of a finished render generation. */
  fetchFrame(generation: number): Promise<ArrayBuffer>;
}
