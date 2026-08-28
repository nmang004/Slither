import json, sys, collections
# --- Page-defect backbone: the page ITSELF is defective -----------------------
# Weighted like Ahrefs' health score, which is the only published formula:
# health tracks the share of pages that are actually broken.
PAGE_CRITICAL = {'internal_server_error','internal_client_error','internal_no_response',
 'internal_redirect_loop','non_indexable_target','conflicting_directives'}
PAGE_HIGH = {'PageTitles:missing','exact_duplicate','soft_404','http_urls','mixed_content',
 'insecure_form_action','form_on_http','orphan_pages'}
# --- Bounded penalties: site-level or link-level, not a defective page --------
SITE_HIGH = {'broken_internal','no_sitemap_found','multiple_conflicting'}
SITE_MED = {'PageTitles:duplicate','PageTitles:multiple_tags','PageTitles:outside_head',
 'Canonicals:missing','canonical_relative','internal_redirect_chain','internal_access_restricted',
 'missing_return_links','incorrect_lang_codes','non_200_urls','non_canonical_urls','noindex_urls',
 'multiple_entries','cwv_lcp_poor','cwv_inp_poor','cwv_cls_poor','cwv_score_poor',
 'low_content','slow_server_response','no_internal_outlinks','non_indexable_in_sitemap','over_50k_urls',
 'over_50mb','parse_errors','js_injected_title','js_injected_description','js_injected_canonical',
 'js_injected_h1','js_injected_structured_data','localhost_outlinks','contains_space'}
NOT_SCORED = {'missing_hsts','missing_csp','missing_x_content_type_options',
 'missing_x_frame_options','missing_referrer_policy','unsafe_cross_origin',
 'ai_crawlers_blocked','noai_directive','has_parameters','ga_tracking_params'}
CRIT_WEIGHT=100.0   # a broken page costs its full share of the site
HIGH_WEIGHT=55.0    # a materially defective page costs about half that
W={'site_high':12.0,'site_med':4.0,'low':1.0}
B={'site_high':20.0,'site_med':8.0,'low':5.0}
FLOOR=0.12
def bucket(cat,chk):
    if chk in NOT_SCORED: return None
    q=f"{cat}:{chk}"
    if chk in PAGE_CRITICAL: return 'page_critical'
    if chk in PAGE_HIGH or q in PAGE_HIGH: return 'page_high'
    if chk in SITE_HIGH or q in SITE_HIGH: return 'site_high'
    if chk in SITE_MED or q in SITE_MED: return 'site_med'
    return 'low'
def score(path, verbose=False):
    d=json.load(open(path)); total=max(d['summary']['total_pages'],1)
    # A proportion measured over one or two pages is noise, so the denominator
    # has a floor: one bad page on a tiny crawl is not "100% of the site".
    denom=max(total,5)
    crit=set(); high=set(); sums=collections.defaultdict(float)
    for i in d['issues']['issues']:
        b=bucket(i['category'], i['check'])
        if b is None: continue
        urls={u['url'] for u in i['urls']}
        if b=='page_critical':
            crit|=urls or {f'__site__{i["check"]}'}
        elif b=='page_high':
            high|=urls or {f'__site__{i["check"]}'}
        else:
            aff=len(i['urls'])
            cov=1.0 if aff==0 else max(FLOOR, min(1.0,(aff/total)**0.5))
            sums[b]+=W[b]*cov
    high-=crit  # a page already counted as broken is not penalised twice
    ded=CRIT_WEIGHT*min(1.0,len(crit)/denom) + HIGH_WEIGHT*min(1.0,len(high)/denom)
    ded+=sum(min(sums[b],B[b]) for b in sums)
    s=max(0,int(round(100-ded)))
    if verbose:
        print(f"      broken {len(crit)}/{total} -{CRIT_WEIGHT*len(crit)/denom:.1f} | "
              f"defective {len(high)}/{total} -{HIGH_WEIGHT*len(high)/denom:.1f}"
              +"".join(f" | {b} -{min(sums[b],B[b]):.1f}" for b in ['site_high','site_med','low'] if b in sums))
    return s,total
def band(s): return 'A' if s>=90 else 'B' if s>=80 else 'C' if s>=70 else 'D' if s>=60 else 'F'
