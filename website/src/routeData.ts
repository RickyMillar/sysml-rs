import { defineRouteMiddleware } from '@astrojs/starlight/route-data';

/** Global pre-alpha notice; a page can override it with its own banner. */
export const onRequest = defineRouteMiddleware((context) => {
  const { entry } = context.locals.starlightRoute;
  entry.data.banner ??= {
    content:
      'sysml-rs is <strong>pre-alpha</strong>: a partial SysML v2 implementation with no OMG conformance claim. Interfaces change without deprecation while the version stays 0.x.',
  };
});
