// What a hosting platform is called to the viewer.

import type { PlatformKind } from "../generated/PlatformKind";

const NAMES: Record<PlatformKind, string> = { github: "GitHub", azureDevOps: "Azure DevOps" };

export const platformName = (kind: PlatformKind): string => NAMES[kind];
