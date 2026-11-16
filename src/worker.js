const json = (data, status = 200) => new Response(JSON.stringify(data), {
  status,
  headers: {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  },
});

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === "/api/health") {
      return json({
        ok: true,
        service: "meridian-api",
        storage: "not-configured",
        message: "Connect an external database before enabling bookings.",
      });
    }

    if (url.pathname.startsWith("/api/")) {
      return json({
        error: "Booking services are not configured yet.",
        code: "BACKEND_NOT_CONFIGURED",
      }, 503);
    }

    return env.ASSETS.fetch(request);
  },
};
