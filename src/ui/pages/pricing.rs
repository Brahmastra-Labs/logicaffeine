use dioxus::prelude::*;
use crate::ui::router::Route;
use crate::ui::components::main_nav::{MainNav, ActivePage};

const PRICING_STYLE: &str = r#"
* { box-sizing: border-box; }
a { color: inherit; }

.pricing {
  height: 100vh;
  color: var(--text-primary);
  background:
    radial-gradient(1200px 600px at 50% -120px, rgba(167,139,250,0.18), transparent 60%),
    radial-gradient(900px 500px at 15% 30%, rgba(96,165,250,0.18), transparent 60%),
    radial-gradient(800px 450px at 90% 45%, rgba(34,197,94,0.10), transparent 62%),
    linear-gradient(180deg, #070a12, #0b1022 55%, #070a12);
  overflow-x: hidden;
  overflow-y: auto;
  font-family: var(--font-sans);
  position: relative;
}

.bg-orb {
  position: absolute;
  inset: auto;
  width: 520px;
  height: 520px;
  border-radius: var(--radius-full);
  filter: blur(42px);
  opacity: 0.22;
  pointer-events: none;
  animation: float 14s ease-in-out infinite, pulse-glow 10s ease-in-out infinite;
}
.orb1 { top: -220px; left: -160px; background: radial-gradient(circle at 30% 30%, var(--color-accent-blue), transparent 60%); animation-delay: 0s; }
.orb2 { top: 120px; right: -200px; background: radial-gradient(circle at 40% 35%, var(--color-accent-purple), transparent 60%); animation-delay: -5s; }
.orb3 { bottom: -260px; left: 20%; background: radial-gradient(circle at 40% 35%, rgba(34,197,94,0.9), transparent 60%); animation-delay: -10s; }

@keyframes float {
  0%, 100% { transform: translate3d(0, 0, 0); }
  50% { transform: translate3d(0, -20px, 0); }
}

@keyframes pulse-glow {
  0%, 100% { opacity: 0.22; }
  50% { opacity: 0.32; }
}

@keyframes fadeInUp {
  from { opacity: 0; transform: translateY(24px); }
  to { opacity: 1; transform: translateY(0); }
}

.pricing-container {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 60px var(--spacing-xl);
  max-width: 1000px;
  margin: 0 auto;
}

.pricing-header {
  text-align: center;
  margin-bottom: 50px;
  animation: fadeInUp 0.6s ease both;
}

.pricing-header h1 {
  font-size: var(--font-display-lg);
  font-weight: 900;
  letter-spacing: -2px;
  background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 65%, rgba(229,231,235,0.62) 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  margin-bottom: var(--spacing-lg);
}

.pricing-header p {
  color: var(--text-secondary);
  font-size: var(--font-body-lg);
  line-height: 1.65;
}

.pricing-tiers {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: var(--spacing-xl);
  width: 100%;
  margin-bottom: 40px;
}

.tier-card {
  position: relative;
  background: rgba(255,255,255,0.04);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-xl);
  padding: var(--spacing-xxl);
  display: flex;
  flex-direction: column;
  backdrop-filter: blur(18px);
  transition: transform 0.18s ease, border-color 0.18s ease, background 0.18s ease;
  overflow: hidden;
  animation: fadeInUp 0.6s ease both;
}

.tier-card:nth-child(1) { animation-delay: 0.1s; }
.tier-card:nth-child(2) { animation-delay: 0.15s; }
.tier-card:nth-child(3) { animation-delay: 0.2s; }
.tier-card:nth-child(4) { animation-delay: 0.25s; }
.tier-card:nth-child(5) { animation-delay: 0.3s; }

.tier-card::before {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: var(--radius-xl);
  background: linear-gradient(135deg, rgba(96,165,250,0.12), rgba(167,139,250,0.12));
  opacity: 0;
  transition: opacity 0.3s ease;
  pointer-events: none;
}

.tier-card:hover {
  transform: translateY(-3px);
  border-color: rgba(167,139,250,0.28);
  background: rgba(255,255,255,0.06);
}

.tier-card:hover::before {
  opacity: 1;
}

.tier-card.supporter {
  border-color: rgba(167,139,250,0.35);
  background: linear-gradient(135deg, rgba(167,139,250,0.08) 0%, rgba(96,165,250,0.06) 100%);
}

.tier-card.disabled {
  opacity: 0.4;
  pointer-events: none;
  filter: grayscale(0.5);
}

.tier-card.disabled:hover {
  transform: none;
  border-color: rgba(255,255,255,0.10);
  background: rgba(255,255,255,0.04);
}

.tier-card.disabled::before {
  display: none;
}

.tier-card.disabled .btn-primary,
.tier-card.disabled .btn-secondary,
.tier-card.disabled .btn-contact {
  background: rgba(255,255,255,0.08);
  cursor: not-allowed;
  box-shadow: none;
}

.free-license-banner {
  position: relative;
  background: rgba(255,255,255,0.04);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-xl);
  padding: var(--spacing-xxl);
  margin-bottom: 40px;
  width: 100%;
  text-align: center;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.05s both;
}

.free-license-banner.disabled {
  opacity: 0.4;
  pointer-events: none;
  filter: grayscale(0.5);
}

.free-license-banner h2 {
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  margin-bottom: var(--spacing-md);
  font-weight: 700;
}

.free-license-banner p {
  color: var(--text-secondary);
  margin-bottom: var(--spacing-xl);
  line-height: 1.65;
}

.free-license-banner .btn-free {
  display: inline-block;
  background: linear-gradient(135deg, rgba(96,165,250,0.95), rgba(167,139,250,0.95));
  color: #060814;
  padding: var(--spacing-md) var(--spacing-xxl);
  border-radius: var(--radius-lg);
  font-size: var(--font-body-md);
  font-weight: 650;
  text-decoration: none;
  transition: all 0.2s ease;
  box-shadow: 0 18px 40px rgba(96,165,250,0.18);
}

.free-license-banner .btn-free:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 20px rgba(96,165,250,0.4);
}

.tier-badge {
  display: inline-block;
  background: linear-gradient(135deg, var(--color-accent-blue), var(--color-accent-purple));
  color: #060814;
  font-size: var(--font-caption-md);
  font-weight: 700;
  padding: 5px var(--spacing-md);
  border-radius: var(--radius-full);
  margin-bottom: var(--spacing-lg);
  align-self: flex-start;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.early-access-badge {
  display: inline-block;
  background: linear-gradient(135deg, var(--color-success), #16a34a);
  color: #060814;
  font-size: var(--font-caption-sm);
  font-weight: 700;
  padding: var(--spacing-xs) 10px;
  border-radius: var(--radius-full);
  margin-bottom: var(--spacing-md);
  align-self: flex-start;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.coming-soon-badge {
  display: inline-block;
  background: rgba(255,255,255,0.12);
  color: var(--text-secondary);
  font-size: var(--font-caption-sm);
  font-weight: 700;
  padding: var(--spacing-xs) 10px;
  border-radius: var(--radius-full);
  margin-bottom: var(--spacing-md);
  align-self: flex-start;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.tier-name {
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  font-weight: 700;
  margin-bottom: var(--spacing-sm);
}

.tier-revenue {
  color: var(--text-secondary);
  font-size: var(--font-caption-lg);
  margin-bottom: var(--spacing-xl);
}

.tier-price {
  margin-bottom: var(--spacing-sm);
}

.tier-price .amount {
  color: var(--text-primary);
  font-size: var(--font-display-md);
  font-weight: 800;
}

.tier-price .period {
  color: var(--text-secondary);
  font-size: var(--font-body-md);
}

.tier-annual {
  color: var(--color-accent-purple);
  font-size: var(--font-caption-lg);
  margin-bottom: var(--spacing-xl);
}

.tier-features {
  list-style: none;
  padding: 0;
  margin: 0 0 var(--spacing-xl) 0;
  flex-grow: 1;
}

.tier-features li {
  color: var(--text-secondary);
  font-size: var(--font-caption-lg);
  padding: var(--spacing-sm) 0;
  padding-left: var(--spacing-xl);
  position: relative;
  line-height: 1.5;
}

.tier-features li::before {
  content: "✓";
  position: absolute;
  left: 0;
  color: var(--color-accent-purple);
}

.tier-buttons {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
}

.btn-primary {
  display: block;
  background: linear-gradient(135deg, rgba(96,165,250,0.95), rgba(167,139,250,0.95));
  color: #060814;
  padding: var(--spacing-md) var(--spacing-xl);
  border-radius: var(--radius-lg);
  font-size: var(--font-body-md);
  font-weight: 650;
  text-decoration: none;
  text-align: center;
  transition: all 0.2s ease;
  box-shadow: 0 18px 40px rgba(96,165,250,0.18);
}

.btn-primary:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 20px rgba(96,165,250,0.4);
}

.btn-secondary {
  display: block;
  background: rgba(255,255,255,0.05);
  color: var(--color-accent-purple);
  padding: var(--spacing-md) var(--spacing-xl);
  border: 1px solid rgba(167,139,250,0.3);
  border-radius: var(--radius-lg);
  font-size: var(--font-caption-lg);
  font-weight: 600;
  text-decoration: none;
  text-align: center;
  transition: all 0.2s ease;
}

.btn-secondary:hover {
  background: rgba(167,139,250,0.1);
  border-color: rgba(167,139,250,0.5);
}

.btn-contact {
  display: block;
  background: rgba(255,255,255,0.06);
  color: var(--text-primary);
  padding: var(--spacing-md) var(--spacing-xl);
  border-radius: var(--radius-lg);
  border: 1px solid rgba(255,255,255,0.10);
  font-size: var(--font-body-md);
  font-weight: 600;
  text-decoration: none;
  text-align: center;
  transition: all 0.2s ease;
}

.btn-contact:hover {
  background: rgba(255,255,255,0.10);
  border-color: rgba(255,255,255,0.14);
}

.lifetime-section {
  position: relative;
  background: linear-gradient(135deg, rgba(167,139,250,0.12) 0%, rgba(96,165,250,0.08) 100%);
  border: 1px solid rgba(167,139,250,0.3);
  border-radius: var(--radius-xl);
  padding: 40px;
  text-align: center;
  width: 100%;
  margin-bottom: 40px;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.1s both;
  overflow: hidden;
}

.lifetime-section::before {
  content: "";
  position: absolute;
  inset: 0;
  background: radial-gradient(600px 300px at 50% 0%, rgba(167,139,250,0.15), transparent 70%);
  pointer-events: none;
}

.lifetime-section h2 {
  position: relative;
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  font-weight: 700;
  margin-bottom: var(--spacing-md);
}

.lifetime-section .price {
  position: relative;
  color: var(--color-accent-purple);
  font-size: 42px;
  font-weight: 800;
  margin-bottom: var(--spacing-sm);
}

.lifetime-section .subtext {
  position: relative;
  color: var(--text-secondary);
  font-size: var(--font-caption-lg);
  margin-bottom: var(--spacing-xl);
}

.license-section {
  background: rgba(255,255,255,0.04);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-xl);
  padding: 40px;
  margin-bottom: 40px;
  width: 100%;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.35s both;
}

.license-section h2 {
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  font-weight: 700;
  margin-bottom: var(--spacing-xl);
}

.license-section h3 {
  color: var(--color-accent-purple);
  font-size: var(--font-body-lg);
  font-weight: 600;
  margin: var(--spacing-xl) 0 var(--spacing-md) 0;
}

.license-section p {
  color: var(--text-secondary);
  line-height: 1.8;
  margin-bottom: var(--spacing-lg);
}

.license-section ul {
  color: var(--text-secondary);
  line-height: 1.8;
  margin-left: var(--spacing-xl);
  margin-bottom: var(--spacing-lg);
}

.license-section li {
  margin-bottom: var(--spacing-sm);
}

.manage-section {
  background: rgba(255,255,255,0.03);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-xl);
  padding: var(--spacing-xxl);
  text-align: center;
  width: 100%;
  margin-bottom: 40px;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.3s both;
}

.manage-section p {
  color: var(--text-secondary);
  margin-bottom: var(--spacing-lg);
  line-height: 1.65;
}

.contact-section {
  background: linear-gradient(135deg, rgba(96,165,250,0.08) 0%, rgba(167,139,250,0.08) 100%);
  border: 1px solid rgba(167,139,250,0.25);
  border-radius: var(--radius-xl);
  padding: 40px;
  text-align: center;
  width: 100%;
  backdrop-filter: blur(18px);
  animation: fadeInUp 0.6s ease 0.4s both;
}

.contact-section h2 {
  color: var(--text-primary);
  font-size: var(--font-heading-lg);
  font-weight: 700;
  margin-bottom: var(--spacing-lg);
}

.contact-section p {
  color: var(--text-secondary);
  margin-bottom: var(--spacing-xl);
  line-height: 1.65;
}

.contact-links {
  display: flex;
  gap: var(--spacing-lg);
  justify-content: center;
  flex-wrap: wrap;
}

.contact-email {
  display: inline-block;
  background: linear-gradient(135deg, rgba(96,165,250,0.95), rgba(167,139,250,0.95));
  color: #060814;
  padding: var(--spacing-md) var(--spacing-xxl);
  border-radius: var(--radius-lg);
  font-size: var(--font-body-md);
  font-weight: 650;
  text-decoration: none;
  transition: all 0.2s ease;
  box-shadow: 0 18px 40px rgba(96,165,250,0.18);
}

.contact-email:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 20px rgba(96,165,250,0.4);
}

.back-link {
  margin-top: 40px;
  background: rgba(255,255,255,0.05);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-lg);
  padding: var(--spacing-md) var(--spacing-xl);
  color: var(--text-secondary);
  font-size: var(--font-body-sm);
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
}

.back-link:hover {
  background: rgba(255,255,255,0.08);
  color: var(--text-primary);
  border-color: rgba(255,255,255,0.14);
}

.pricing-footer-links {
  display: flex;
  gap: var(--spacing-md);
  align-items: center;
  margin-top: 40px;
}

.github-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--spacing-sm);
  background: rgba(255,255,255,0.05);
  border: 1px solid rgba(255,255,255,0.10);
  border-radius: var(--radius-lg);
  padding: var(--spacing-md) var(--spacing-xl);
  color: var(--text-secondary);
  font-size: var(--font-body-sm);
  font-weight: 600;
  text-decoration: none;
  transition: all 0.2s ease;
}

.github-btn:hover {
  background: rgba(255,255,255,0.08);
  color: var(--text-primary);
  border-color: rgba(255,255,255,0.14);
}

.github-btn svg {
  width: 18px;
  height: 18px;
  fill: currentColor;
}

@media (max-width: 700px) {
  .pricing-header h1 {
    font-size: var(--font-display-md);
  }
  .pricing-tiers {
    grid-template-columns: 1fr;
  }
}

@media (prefers-reduced-motion: reduce) {
  * { transition: none !important; animation: none !important; }
}
"#;

const STRIPE_FREE_LICENSE: &str = "https://buy.stripe.com/9B63cx77ZgB5cKu40Ue3e06";
const STRIPE_SUPPORTER_MONTHLY: &str = "https://buy.stripe.com/5kQbJ33VN5Wr25Q8hae3e05";
const STRIPE_PRO_MONTHLY: &str = "https://buy.stripe.com/eVq00lgIzckPbGqcxqe3e03";
const STRIPE_PRO_ANNUAL: &str = "https://buy.stripe.com/4gM3cxakb0C76m69lee3e04";
const STRIPE_PREMIUM_MONTHLY: &str = "https://buy.stripe.com/dRm4gB9g73OjfWG2WQe3e01";
const STRIPE_PREMIUM_ANNUAL: &str = "https://buy.stripe.com/5kQ9AVcsjfx1h0K54Ye3e02";
const STRIPE_LIFETIME: &str = "https://buy.stripe.com/8x200l3VN98D7qa1SMe3e00";
const STRIPE_CUSTOMER_PORTAL: &str = "https://billing.stripe.com/p/login/8x200l3VN98D7qa1SMe3e00";

#[component]
pub fn Pricing() -> Element {
    rsx! {
        style { "{PRICING_STYLE}" }

        div { class: "pricing",
            div { class: "bg-orb orb1" }
            div { class: "bg-orb orb2" }
            div { class: "bg-orb orb3" }

            MainNav { active: ActivePage::Pricing }

            div { class: "pricing-container",
                div { class: "pricing-header",
                    h1 { "Commercial Licensing" }
                    p { "Business Source License — free for individuals and small teams" }
                }

                div { class: "free-license-banner disabled",
                    h2 { "Free for Small Teams" }
                    p {
                        "Individuals and organizations with fewer than 25 employees can use LOGOS at no cost. "
                        "Get a free license to track your usage and unlock all features."
                    }
                    a {
                        class: "btn-free",
                        href: STRIPE_FREE_LICENSE,
                        target: "_blank",
                        "Get Free License"
                    }
                }

                div { class: "lifetime-section",
                    span { class: "early-access-badge", "Early Access Pricing" }
                    h2 { "Lifetime License" }
                    div { class: "price", "$50/seat" }
                    div { class: "subtext", "One-time payment. Permanent license with Z3 Static Verification." }
                    a {
                        class: "btn-primary",
                        href: STRIPE_LIFETIME,
                        target: "_blank",
                        "Buy Lifetime License"
                    }
                }

                div { class: "pricing-tiers",
                    div { class: "tier-card supporter",
                        span { class: "early-access-badge", "Early Access Pricing" }
                        div { class: "tier-name", "Supporter" }
                        div { class: "tier-revenue", "For individuals and hobbyists" }
                        div { class: "tier-price",
                            span { class: "amount", "$5" }
                            span { class: "period", " /month" }
                        }
                        div { class: "tier-annual", "Optional - personal use is free" }
                        ul { class: "tier-features",
                            li { "Support LOGOS development" }
                            li { "Personal/hobbyist use" }
                            li { "Core feature access" }
                        }
                        div { class: "tier-buttons",
                            a {
                                class: "btn-primary",
                                href: STRIPE_SUPPORTER_MONTHLY,
                                target: "_blank",
                                "Become a Supporter"
                            }
                        }
                    }

                    div { class: "tier-card disabled",
                        span { class: "coming-soon-badge", "Coming Soon" }
                        div { class: "tier-name", "Pro" }
                        div { class: "tier-revenue", "For organizations with 25-100 employees" }
                        div { class: "tier-price",
                            span { class: "amount", "$25" }
                            span { class: "period", " /seat/month" }
                        }
                        div { class: "tier-annual", "or $240/seat/year (save 20%)" }
                        ul { class: "tier-features",
                            li { "Commercial use license" }
                            li { "Z3 Static Verification" }
                            li { "Full feature access" }
                            li { "Regular updates" }
                        }
                        div { class: "tier-buttons",
                            a {
                                class: "btn-primary",
                                href: STRIPE_PRO_MONTHLY,
                                target: "_blank",
                                "Subscribe Monthly"
                            }
                            a {
                                class: "btn-secondary",
                                href: STRIPE_PRO_ANNUAL,
                                target: "_blank",
                                "Subscribe Annually"
                            }
                        }
                    }

                    div { class: "tier-card disabled",
                        span { class: "coming-soon-badge", "Coming Soon" }
                        div { class: "tier-name", "Premium" }
                        div { class: "tier-revenue", "For organizations with 100-500 employees" }
                        div { class: "tier-price",
                            span { class: "amount", "$50" }
                            span { class: "period", " /seat/month" }
                        }
                        div { class: "tier-annual", "or $480/seat/year (save 20%)" }
                        ul { class: "tier-features",
                            li { "Everything in Pro" }
                            li { "Early access to new features" }
                            li { "Custom integrations" }
                        }
                        div { class: "tier-buttons",
                            a {
                                class: "btn-primary",
                                href: STRIPE_PREMIUM_MONTHLY,
                                target: "_blank",
                                "Subscribe Monthly"
                            }
                            a {
                                class: "btn-secondary",
                                href: STRIPE_PREMIUM_ANNUAL,
                                target: "_blank",
                                "Subscribe Annually"
                            }
                        }
                    }

                    div { class: "tier-card disabled",
                        span { class: "coming-soon-badge", "Coming Soon" }
                        div { class: "tier-name", "Enterprise" }
                        div { class: "tier-revenue", "For organizations with 500+ employees" }
                        div { class: "tier-price",
                            span { class: "amount", "Custom" }
                        }
                        div { class: "tier-annual", "Tailored to your needs" }
                        ul { class: "tier-features",
                            li { "Everything in Premium" }
                            li { "On-premise deployment options" }
                            li { "Volume discounts" }
                        }
                        div { class: "tier-buttons",
                            a {
                                class: "btn-contact",
                                href: "mailto:tristen@brahmastra-labs.com",
                                "Contact Sales"
                            }
                        }
                    }

                    div { class: "tier-card disabled",
                        span { class: "coming-soon-badge", "Coming Soon" }
                        div { class: "tier-name", "Support Plans" }
                        div { class: "tier-revenue", "Technical support available separately" }
                        div { class: "tier-price",
                            span { class: "amount", "Custom" }
                        }
                        div { class: "tier-annual", "Tailored to your needs" }
                        ul { class: "tier-features",
                            li { "Priority email support" }
                            li { "Dedicated support" }
                            li { "Custom SLAs" }
                            li { "Training and onboarding" }
                        }
                        div { class: "tier-buttons",
                            a {
                                class: "btn-contact",
                                href: "mailto:tristen@brahmastra-labs.com",
                                "Contact for Pricing"
                            }
                        }
                    }

                    div { class: "tier-card disabled",
                        span { class: "coming-soon-badge", "Coming Soon" }
                        div { class: "tier-name", "Semantic Tokenizer" }
                        div { class: "tier-revenue", "For AI model training" }
                        div { class: "tier-price",
                            span { class: "amount", "Custom" }
                        }
                        div { class: "tier-annual", "Contact us for pricing" }
                        ul { class: "tier-features",
                            li { "License for AI model training" }
                            li { "Commercial training data rights" }
                            li { "Custom volume pricing" }
                        }
                        div { class: "tier-buttons",
                            a {
                                class: "btn-contact",
                                href: "mailto:tristen@brahmastra-labs.com",
                                "Contact for Pricing"
                            }
                        }
                    }
                }

                div { class: "manage-section",
                    p { "Already a subscriber? Manage your subscription, update payment methods, or view invoices." }
                    a {
                        class: "btn-secondary",
                        href: STRIPE_CUSTOMER_PORTAL,
                        target: "_blank",
                        "Manage Subscription"
                    }
                }

                div { class: "license-section",
                    h2 { "Business Source License" }

                    p {
                        "LOGOS is released under the Business Source License 1.1. The source code is "
                        "publicly available, and the software is free to use for individuals and small teams."
                    }

                    h3 { "Free Use" }
                    p { "You may use LOGOS at no cost if you are:" }
                    ul {
                        li { "An individual" }
                        li { "An organization with fewer than 25 employees" }
                    }

                    h3 { "Commercial License Required" }
                    p {
                        "If your organization has 25 or more employees and you wish to use "
                        "LOGOS as a Logic Service, a commercial license is required. Select a tier above "
                        "based on your organization's size."
                    }

                    h3 { "Open Source Transition" }
                    p {
                        "On December 24, 2029, LOGOS will transition to the MIT License, "
                        "making it fully open source."
                    }
                }

                div { class: "contact-section",
                    h2 { "Get in Touch" }
                    p { "Questions about licensing, support contracts, or enterprise needs?" }
                    div { class: "contact-links",
                        a {
                            class: "contact-email",
                            href: "mailto:tristen@brahmastra-labs.com",
                            "Enterprise Sales"
                        }
                        a {
                            class: "contact-email",
                            href: "mailto:tristen@brahmastra-labs.com",
                            "Support Inquiries"
                        }
                    }
                }

                div { class: "pricing-footer-links",
                    a {
                        href: "https://github.com/Brahmastra-Labs/logicaffeine",
                        target: "_blank",
                        class: "github-btn",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            path {
                                d: "M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z"
                            }
                        }
                        "GitHub"
                    }
                    Link {
                        class: "back-link",
                        to: Route::Landing {},
                        "← Back"
                    }
                }
            }
        }
    }
}
