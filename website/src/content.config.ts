import { defineCollection, z } from 'astro:content';
import { docsLoader } from '@astrojs/starlight/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

/**
 * Scope labels from docs/documentation-policy.md. Every page that makes a
 * language, implementation, or tooling claim must carry at least one.
 */
export const SCOPE_LABELS = [
  'SysML v2 / KerML',
  'sysml-rs implementation',
  'sysml-rs tooling',
  'OMG API subset',
  'Experimental / partial support',
] as const;

export const collections = {
  docs: defineCollection({
    loader: docsLoader(),
    schema: docsSchema({
      extend: z.object({
        /** One or more scope labels from the documentation policy. */
        scope: z.array(z.enum(SCOPE_LABELS)).optional(),
        /** Maturity of the documented surface, per the documentation policy. */
        status: z.enum(['pre-alpha', 'experimental', 'partial', 'stable']).optional(),
        /** Release tag, commit, catalogue revision, or dated standard reference. */
        last_verified_against: z.string().optional(),
        /** Code, test, spec clause, or generated artefact backing the page. */
        source_of_truth: z.array(z.string()).optional(),
        /** Route or URL of the relevant known-limitations entry. */
        known_limitations: z.string().optional(),
      }),
    }),
  }),
};
