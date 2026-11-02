// modfence: layer=app

import fs, { readFile as rf, type Stats } from "node:fs";
import * as path from 'path';
import "./polyfill";
import { helper } from "./util.js"; // modfence: ignore[layers] reason="legacy"
import legacy = require("./legacy");
export { Shape as PublicShape } from "./shapes";
export * from "./all";
export * as ns from "./ns";

// modfence: layer=service
export function load(name: string): string;
export function load(name: number): string;
export function load(name: string | number): string {
  return helper(String(name)) + path.join("a", "b");
}

/** The repository. */
// modfence: owner=team-a
/**
 * Spans lines.
 */
@sealed
export class Repo extends Base {
  static create(): Repo {
    return new Repo();
  }
  items = [];
  handler = () => this.run();
  // modfence: role=entry
  run(): void {
    load("x").trim();
    rf?.("y");
  }
  get size(): number {
    return 1;
  }
  set size(value: number) {}
  #secret(): void {}
  "odd.name"(): void {}
  [Symbol.iterator](): void {}
}

export namespace Outer.Inner {
  export const answer = 42;
  export function deep(): void {
    helper("x");
  }
}

export interface Config {
  a: string;
}
export interface Config {
  b: number;
}
export type Id = string | number;
export enum Color {
  Red,
  Green,
}
export const arrow = (x: number) => x + 1;
const viaFunction = function () {
  return arrow(1);
};
let counter = 0;
const Local = class {};

const trailing = 1; // modfence: layer=ignored
const lazy = () => import("./lazy");
const legacyLoad = () => require("./legacy-cjs");
