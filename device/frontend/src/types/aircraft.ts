export type AircraftAction =
  "takeoff" | "land" | "cancel_landing" | "return_home" | "cancel_return_home";

export interface AircraftOutcome {
  action: AircraftAction;
  /** The aircraft answered the command */
  acknowledged: boolean;
  /** How often it went out: again only while there was no answer */
  sends: number;
  /** The answer's payload, as hex */
  reply: string | null;
}
