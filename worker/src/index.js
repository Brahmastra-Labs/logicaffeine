import Stripe from 'stripe';

const CORS_HEADERS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type',
};

export default {
  async fetch(request, env) {
    if (request.method === 'OPTIONS') {
      return new Response(null, { headers: CORS_HEADERS });
    }

    const url = new URL(request.url);

    if (url.pathname === '/validate' && request.method === 'POST') {
      return handleValidate(request, env);
    }

    if (url.pathname === '/session' && request.method === 'POST') {
      return handleSession(request, env);
    }

    if (url.pathname === '/health') {
      return new Response(JSON.stringify({ status: 'ok' }), {
        headers: { ...CORS_HEADERS, 'Content-Type': 'application/json' },
      });
    }

    return new Response(JSON.stringify({ error: 'Not found' }), {
      status: 404,
      headers: { ...CORS_HEADERS, 'Content-Type': 'application/json' },
    });
  },
};

async function handleValidate(request, env) {
  try {
    const { licenseKey } = await request.json();

    if (!licenseKey) {
      return jsonResponse({ valid: false, error: 'No license key provided' }, 400);
    }

    if (!licenseKey.startsWith('sub_')) {
      return jsonResponse({ valid: false, error: 'Invalid license key format' }, 400);
    }

    const stripe = new Stripe(env.STRIPE_SECRET_KEY, {
      apiVersion: '2023-10-16',
      httpClient: Stripe.createFetchHttpClient(),
    });

    const subscription = await stripe.subscriptions.retrieve(licenseKey, {
      expand: ['items.data.price.product'],
    });

    const isActive = subscription.status === 'active' || subscription.status === 'trialing';
    const priceData = subscription.items.data[0]?.price;
    const productData = priceData?.product;

    const plan = determinePlan(priceData, productData);

    return jsonResponse({
      valid: isActive,
      status: subscription.status,
      plan: plan,
      customerId: subscription.customer,
      currentPeriodEnd: subscription.current_period_end,
      cancelAtPeriodEnd: subscription.cancel_at_period_end,
    });
  } catch (error) {
    if (error.type === 'StripeInvalidRequestError') {
      return jsonResponse({ valid: false, error: 'License key not found' }, 404);
    }

    console.error('Validation error:', error);
    return jsonResponse({ valid: false, error: 'Validation failed' }, 500);
  }
}

function determinePlan(priceData, productData) {
  const lookupKey = priceData?.lookup_key;
  if (lookupKey) return lookupKey;

  const productName = productData?.name?.toLowerCase() || '';

  if (productName.includes('free')) return 'free';
  if (productName.includes('supporter')) return 'supporter';
  if (productName.includes('premium')) return 'premium';
  if (productName.includes('pro')) return 'pro';
  if (productName.includes('lifetime')) return 'lifetime';
  if (productName.includes('enterprise')) return 'enterprise';

  const amount = priceData?.unit_amount || 0;
  if (amount === 0) return 'free';
  if (amount <= 500) return 'supporter';
  if (amount <= 2500) return 'pro';
  if (amount <= 5000) return 'premium';
  if (amount >= 29900) return 'lifetime';

  return 'unknown';
}

async function handleSession(request, env) {
  try {
    const { sessionId } = await request.json();

    if (!sessionId) {
      return jsonResponse({ error: 'No session ID provided' }, 400);
    }

    if (!sessionId.startsWith('cs_')) {
      return jsonResponse({ error: 'Invalid session ID format' }, 400);
    }

    const stripe = new Stripe(env.STRIPE_SECRET_KEY, {
      apiVersion: '2023-10-16',
      httpClient: Stripe.createFetchHttpClient(),
    });

    const session = await stripe.checkout.sessions.retrieve(sessionId, {
      expand: ['subscription', 'subscription.items.data.price.product'],
    });

    if (!session.subscription) {
      return jsonResponse({ error: 'No subscription found for this session' }, 404);
    }

    const subscription = session.subscription;
    const priceData = subscription.items?.data[0]?.price;
    const productData = priceData?.product;
    const plan = determinePlan(priceData, productData);

    return jsonResponse({
      subscriptionId: subscription.id,
      status: subscription.status,
      plan: plan,
      customerId: session.customer,
      customerEmail: session.customer_details?.email,
    });
  } catch (error) {
    if (error.type === 'StripeInvalidRequestError') {
      return jsonResponse({ error: 'Session not found' }, 404);
    }

    console.error('Session lookup error:', error);
    return jsonResponse({ error: 'Session lookup failed' }, 500);
  }
}

function jsonResponse(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      ...CORS_HEADERS,
      'Content-Type': 'application/json',
    },
  });
}
