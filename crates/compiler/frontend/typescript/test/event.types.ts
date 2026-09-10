// Opaque Event references are admitted values, not source-created authority.
import { Event, type EventRef } from "../src/index.js";

type Payload = { accepted: boolean };
declare const admitted: EventRef<Payload>;
declare const wrongPayload: EventRef<string>;
const Ready = Event<Payload>("event.ready");

void Ready.wait(admitted);
// @ts-expect-error Event.wait requires the complete typed reference.
void Ready.wait();
// @ts-expect-error A string is not an admitted reference.
void Ready.wait("event.ready");
// @ts-expect-error The wire fields alone cannot construct the opaque source type.
void Ready.wait({ event_id: "evt-forged", generation: 1 });
// @ts-expect-error Payload types must agree.
void Ready.wait(wrongPayload);
