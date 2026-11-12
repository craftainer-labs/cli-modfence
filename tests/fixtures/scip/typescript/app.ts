import { Svc, run as go } from "./lib";

export function main(): number {
  const svc = new Svc();
  svc.runIt();
  return go();
}
