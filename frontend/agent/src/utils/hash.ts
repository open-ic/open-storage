import { sha3_256 } from "js-sha3";

export function hashBytes(bytes: ArrayBuffer): Array<number> {
    const hash = sha3_256.create();
    hash.update(bytes);
    return hash.array();
}