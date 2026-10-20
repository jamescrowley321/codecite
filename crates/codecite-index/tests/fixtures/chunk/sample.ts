import { readFile } from "fs";

export interface Claims {
  sub: string;
}

export enum Grant {
  Code,
  Implicit,
}

export class Client {
  constructor(private issuer: string) {}

  validate(token: string): boolean {
    return token.length > 0;
  }
}

export function helper(): number {
  return 1;
}

export const parse = (raw: string): Claims => {
  return JSON.parse(raw);
};
