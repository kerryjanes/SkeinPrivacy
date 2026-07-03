#!/usr/bin/env -S tsx
// Print the distributor PDA for the current program id. The PDA is derived from the id,
// so consumers (e.g. scripts/mainnet-cutover.sh) must resolve it from the SDK — never hardcode.
import { weft } from '@weft/sdk';

const [distributor] = await weft.findDistributorPda();
process.stdout.write(String(distributor));
