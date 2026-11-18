import { describe, expect, it } from "vitest";
import {
  authorName,
  captionYears,
  commonsPage,
  exifMismatch,
  headquartersNamed,
  licenceAllowed,
  licenceUrl,
  originalSource,
  permissionTerms,
  requestNotCovered,
  stripFlickrPlace,
  takenYear,
  userNames,
} from "../scripts/photo-rules.mjs";

// The photo build's gates (scripts/build-photos.mjs applies exactly these functions).

describe("authorName", () => {
  it("keeps a plain name, from Commons HTML", () => {
    expect(authorName('<a href="//commons.wikimedia.org/wiki/User:Coolcaesar">Coolcaesar</a>')).toBe("Coolcaesar");
    expect(authorName("Radosław Drożdżewski (Zwiadowca21)")).toBe("Radosław Drożdżewski (Zwiadowca21)");
    expect(authorName("User:proshob")).toBe("proshob");
    expect(authorName("Bobak Ha&#39;Eri")).toBe("Bobak Ha'Eri");
    expect(authorName("<b>Jane Roe</b><script>x</script>")).toBe("Jane Roe");
  });
  it("refuses what is not a plain name", () => {
    for (const bad of [
      "",
      "Unknown author",
      "anonymous",
      "Own work",
      "The original uploader was King4057 at English Wikipedia.",
      "a Rockwell Automation Employee",
      "Raysonho @ Open Grid Scheduler / Scalable Grid Engine",
      "jane@example.com",
      "https://example.com/me",
      "www.example.com",
      "&lt;b&gt;Jane&lt;/b&gt;",
      "None",
      "x".repeat(61),
    ])
      expect(authorName(bad), bad).toBeNull();
  });
});

describe("stripFlickrPlace", () => {
  it("drops the Flickr importer's ' from <place>' only for a Flickr file", () => {
    expect(stripFlickrPlace("Peter Bond from Philadelphia, USA", true)).toBe("Peter Bond");
    expect(stripFlickrPlace("Parker Hannifin from Global, United States", true)).toBe("Parker Hannifin");
    expect(stripFlickrPlace("Peter Bond from Philadelphia, USA", false)).toBe("Peter Bond from Philadelphia, USA");
    expect(stripFlickrPlace("Coolcaesar", true)).toBe("Coolcaesar");
  });
});

describe("takenYear", () => {
  const now = new Date("2026-11-18T00:00:00Z");
  it("reads the clean forms of DateTimeOriginal", () => {
    expect(takenYear("2022:05:01 10:00:00", now)).toBe(2022);
    expect(takenYear("2022-05-01", now)).toBe(2022);
    expect(takenYear("2009-08", now)).toBe(2009);
    expect(takenYear("Taken on 8 June 2016, 11:08:51", now)).toBe(2016);
    expect(takenYear("28 October 2013", now)).toBe(2013);
    expect(takenYear("14 July 2021, 09:17:09 (according to Exif data)", now)).toBe(2021);
  });
  it("is null for an upload date, free text, a future year or nothing", () => {
    expect(takenYear("1 May 2008 (original upload date)", now)).toBeNull();
    expect(takenYear("circa 1990", now)).toBeNull();
    expect(takenYear("1990s", now)).toBeNull();
    expect(takenYear("2027-01-01", now)).toBeNull();
    expect(takenYear("", now)).toBeNull();
    expect(takenYear("8 Juni 2016", now)).toBeNull();
  });
  it("captionYears finds the years a caption names", () => {
    expect(captionYears("Intel's headquarters at 2200 Mission College Boulevard, in 2023.")).toEqual([2023]);
    expect(captionYears("Capital One 位于弗吉尼亚州的总部大楼，摄于2018年。")).toEqual([2018]);
    expect(captionYears("A Fab 5 building.")).toEqual([]);
  });
});

describe("licences", () => {
  it("allows public domain, CC0, CC BY and CC BY-SA in any version or port, and nothing else", () => {
    for (const ok of ["Public domain", "CC0", "CC0 1.0", "CC BY 2.0", "CC BY 4.0", "CC BY-SA 3.0", "CC BY-SA 3.0 de", "CC BY-SA 2.0 fr"]) expect(licenceAllowed(ok), ok).toBe(true);
    for (const bad of ["CC BY-NC 4.0", "CC BY-ND 2.0", "CC BY-NC-SA 3.0", "GFDL", "Attribution", "Copyrighted free use", "CC BY", "CC BY-SA 4.0 or GFDL", ""])
      expect(licenceAllowed(bad), bad).toBe(false);
  });
  it("rewrites a creativecommons.org licence URL to https and refuses any other host", () => {
    expect(licenceUrl("http://creativecommons.org/licenses/by-sa/3.0/")).toBe("https://creativecommons.org/licenses/by-sa/3.0/");
    expect(licenceUrl("https://creativecommons.org/publicdomain/zero/1.0/deed.en")).toBe("https://creativecommons.org/publicdomain/zero/1.0/deed.en");
    expect(licenceUrl("https://example.com/licenses/by/4.0")).toBeNull();
    expect(licenceUrl("http://example.com/creativecommons.org/")).toBeNull();
    expect(licenceUrl(null)).toBeNull();
  });
  it("accepts only an https Commons file page", () => {
    expect(commonsPage("https://commons.wikimedia.org/wiki/File:Intel_HQ.jpg")).toBe("https://commons.wikimedia.org/wiki/File:Intel_HQ.jpg");
    expect(commonsPage("http://commons.wikimedia.org/wiki/File:Intel_HQ.jpg")).toBeNull();
    expect(commonsPage("https://commons.wikimedia.org/wiki/User:X")).toBeNull();
  });
});

describe("headquartersNamed", () => {
  it("passes a caption without 'headquarters', and one that names a Wikidata headquarters place", () => {
    expect(headquartersNamed("A Target store in Ventura, California.", [])).toBe(true);
    expect(headquartersNamed("Intel's headquarters in Santa Clara, California.", ["Santa Clara", "Santa Clara County"])).toBe(true);
    expect(headquartersNamed("Lennar's headquarters in Miami-Dade County.", ["Miami, Florida"])).toBe(true);
  });
  it("fails one that names another place, or when Wikidata has none", () => {
    expect(headquartersNamed("Broadcom's headquarters in San Jose, California.", ["Palo Alto", "Santa Clara County"])).toBe(false);
    expect(headquartersNamed("Sysco's headquarters in Houston, Texas.", [])).toBe(false);
  });
});

describe("exifMismatch", () => {
  it("passes EXIF that names no one: placeholders, editing software or the camera's own make in Artist", () => {
    expect(exifMismatch("", ["X"])).toBeNull();
    expect(exifMismatch("unknown", ["X"])).toBeNull();
    expect(exifMismatch("COPYRIGHT, 2007", ["X"])).toBeNull();
    expect(exifMismatch("Copyright", ["X"])).toBeNull();
    expect(exifMismatch("CC BY 2.0", ["X"])).toBeNull();
    expect(exifMismatch("Picasa 3.0", ["X"], { tag: "Artist" })).toBeNull();
    expect(exifMismatch("Canon", ["X"], { tag: "Artist", make: "Canon" })).toBeNull();
    expect(exifMismatch("OLYMPUS", ["X"], { tag: "Artist", make: "OLYMPUS OPTICAL CO.,LTD" })).toBeNull();
  });
  it("passes EXIF whose every word is in the credited names (credit, author, Commons account)", () => {
    expect(exifMismatch("©2019 Kenneth C. Zirkel", ["Kenneth C. Zirkel"])).toBeNull();
    expect(exifMismatch("Copyright 2021, G. Edward Johnson. CC-BY", ["G. Edward Johnson"])).toBeNull();
    expect(exifMismatch("Photographer: Dwight Burdette", ["Dwight Burdette"], { tag: "Artist" })).toBeNull();
    expect(exifMismatch("Jeroen Stroes", ["Jeroen Stroes Aviation Photography"], { tag: "Artist" })).toBeNull();
    expect(exifMismatch("Ajay Suresh", ["ajay_suresh"], { tag: "Artist" })).toBeNull();
    // TRI: the Commons account Hiàn is part of the credited set; accents fold.
    expect(exifMismatch("Jason Zhang (Hian)", ["Jason Zhang", ...userNames('<a href="//commons.wikimedia.org/wiki/User:Hi%C3%A0n">Jason Zhang</a>')], { tag: "Artist" })).toBeNull();
  });
  it("names someone else when the EXIF does (TGT: '© 2008 Paul Turang' on a file credited to Skinner2398)", () => {
    expect(exifMismatch("© 2008 Paul Turang", ["Skinner2398"])).toBe("Paul Turang");
    expect(exifMismatch("Paul Turang", ["Skinner2398"], { tag: "Artist" })).toBe("Paul Turang");
  });
  it("counts a company in Copyright, and a brand in Artist unless it is the camera's make (review round 2, S2)", () => {
    for (const v of ["© Apple", "Microsoft", "Photo: Google", "Copyright Sony", "© DJI", "Samsung", "© Adobe"]) expect(exifMismatch(v, ["Coolcaesar"]), v).not.toBeNull();
    expect(exifMismatch("Canon", ["Coolcaesar"], { tag: "Copyright", make: "Canon" })).toBe("Canon");
    expect(exifMismatch("Google", ["Coolcaesar"], { tag: "Artist", make: "Canon" })).toBe("Google");
    expect(exifMismatch("Picasa", ["Coolcaesar"], { tag: "Copyright" })).toBe("Picasa");
  });
  it("flags 'All rights reserved' with no name, and a credited name inside a longer one (review round 2, S2 and S3)", () => {
    expect(exifMismatch("All rights reserved", ["Coolcaesar"])).toBe("All rights reserved");
    expect(exifMismatch("Coolcaesar for Reuters", ["Coolcaesar"], { tag: "Artist" })).toBe("Coolcaesar for Reuters");
    expect(exifMismatch("© 2010 Jane Roe / Getty Images", ["Jane Roe"])).not.toBeNull();
  });
  it("reads the account names an Artist field links to", () => {
    expect(userNames('<a href="https://en.wikipedia.org/wiki/User:Coolcaesar">Coolcaesar</a> at <a href="https://en.wikipedia.org">en.wikipedia</a>')).toEqual(["Coolcaesar"]);
    expect(userNames("Gary Halvorson, Oregon State Archives")).toEqual([]);
  });
});

describe("requestNotCovered", () => {
  it("is empty when the credit shows the requested attribution in full", () => {
    expect(requestNotCovered("Coolcaesar", "Coolcaesar", "CC BY-SA 3.0")).toEqual([]);
    expect(requestNotCovered("© Raimond Spekking / CC BY-SA 4.0 (via Wikimedia Commons)", "© Raimond Spekking", "CC BY-SA 4.0")).toEqual([]);
    expect(requestNotCovered("Gary Halvorson, Oregon State Archives / Cc-by-4.0", "Gary Halvorson, Oregon State Archives", "CC BY 4.0")).toEqual([]);
    expect(requestNotCovered("Coolcaesar at the English Wikipedia", "Coolcaesar at the English Wikipedia", "CC BY-SA 3.0")).toEqual([]);
  });
  it("lists what the credit would leave out: a ©, a web address, another name", () => {
    expect(requestNotCovered("© Raimond Spekking / CC BY-SA 4.0 (via Wikimedia Commons)", "Raimond Spekking", "CC BY-SA 4.0")).toEqual(["©"]);
    expect(requestNotCovered("Arne Müseler / www.arne-mueseler.com", "Arne Müseler", "CC BY-SA 3.0 de")).toEqual(["www", "mueseler", "com"]);
    expect(requestNotCovered("Photo by Jane Roe for Acme", "Jane Roe", "CC BY 4.0")).toEqual(["photo", "for", "acme"]);
  });
});

describe("permissionTerms", () => {
  const box = (name: string, short: string, url: string, sa: boolean, rest = "", attribution = "") =>
    `This file is licensed under the Creative Commons ${name} license.${attribution ? ` Attribution: ${attribution}` : ""} You are free: to share – to copy, distribute and transmit the work to remix – to adapt the work Under the following conditions: attribution – You must give appropriate credit, provide a link to the license, and indicate if changes were made. You may do so in any reasonable manner, but not in any way that suggests the licensor endorses you or your use.${sa ? " share alike – If you remix, transform, or build upon the material, you must distribute your contributions under the same or compatible license as the original." : ""} ${url} ${short} Creative Commons ${name.replace(/ (Unported|International)$/, "")} true true${rest}`;
  const vrt = (rest = "") =>
    `This work is free and may be used by anyone for any purpose. If you wish to use this content, you do not need to request permission as long as you follow any licensing requirements mentioned on this page. The Wikimedia Foundation has received an e-mail confirming that the copyright holder has approved publication under the terms mentioned on this page. This correspondence has been reviewed by a Volunteer Response Team (VRT) member and stored in our permission archive. The correspondence is available to trusted volunteers as ticket #2021030210012287. If you have questions about the archived correspondence, please use the VRT noticeboard. Ticket link: https://ticket.wikimedia.org/otrs/index.pl?Action=AgentTicketZoom&TicketNumber=2021030210012287 Find other files from the same ticket:${rest}`;
  const panoramio =
    " This image, which was originally posted to Panoramio, was automatically reviewed on 7 January 2017 by Panoramio upload bot, who confirmed that it was available on Panoramio under the above license on that date.";

  it("skips Commons' template text exactly: a box for the file's own licence, the review sentence, the {{self}} line, a VRT notice", () => {
    expect(permissionTerms(box("Attribution 4.0 International", "CC BY 4.0", "https://creativecommons.org/licenses/by/4.0", false, "", "Gary Halvorson, Oregon State Archives / Cc-by-4.0"), "CC BY 4.0")).toEqual({
      conditions: "",
      attribution: "Gary Halvorson, Oregon State Archives / Cc-by-4.0",
      conflict: null,
    });
    expect(permissionTerms(box("Attribution-Share Alike 3.0 Unported", "CC BY-SA 3.0", "https://creativecommons.org/licenses/by-sa/3.0", true, panoramio, "BigBear_in_IN"), "CC BY-SA 3.0")).toEqual({
      conditions: "",
      attribution: "BigBear_in_IN",
      conflict: null,
    });
    expect(permissionTerms("Coolcaesar at the English-language Wikipedia, the copyright holder of this work, hereby publishes it under the following license:").conditions).toBe("");
    expect(permissionTerms(vrt(), "CC BY-SA 4.0")).toEqual({ conditions: "", attribution: null, conflict: null });
    expect(permissionTerms("Public domain", "Public domain").conditions).toBe("");
  });
  it("keeps anything after the template text, so it must be read (review round 2, S1)", () => {
    const after = box("Attribution-Share Alike 4.0 International", "CC BY-SA 4.0", "https://creativecommons.org/licenses/by-sa/4.0", true, " Not for commercial use; ask me first.");
    expect(permissionTerms(after, "CC BY-SA 4.0").conditions).toBe(after);
    expect(permissionTerms(vrt(" Not for use in advertising."), "CC BY-SA 4.0").conditions).toContain("Not for use in advertising.");
    expect(permissionTerms("Not for commercial use; Jane, the copyright holder of this work, hereby publishes it under the following license:").conditions).not.toBe("");
    expect(permissionTerms("Please Ask If you Want It").conditions).toBe("Please Ask If you Want It");
  });
  it("reports a conflict when the box or a restated name is another licence (review round 2, S1; MDT)", () => {
    const nc = "This file is licensed under the Creative Commons Attribution-NonCommercial 4.0 International license.";
    expect(permissionTerms(nc, "CC BY 4.0").conflict).not.toBeNull();
    expect(permissionTerms(nc, "CC BY 4.0").conditions).toBe(nc);
    expect(permissionTerms(box("Attribution 4.0 International", "CC BY 4.0", "https://creativecommons.org/licenses/by/4.0", false), "CC BY-SA 4.0").conflict).not.toBeNull();
    expect(permissionTerms("CC-By-SA-3.0", "CC BY 3.0").conflict).not.toBeNull();
    expect(permissionTerms("", "CC BY 3.0")).toEqual({ conditions: "", attribution: null, conflict: null });
  });
});

describe("originalSource", () => {
  it("takes the first https link that leaves Wikimedia, with its title", () => {
    expect(originalSource('<a rel="nofollow" class="external text" href="https://www.flickr.com/photos/pvsbond/1580244122/">IMG_3987</a>')).toEqual({
      url: "https://www.flickr.com/photos/pvsbond/1580244122/",
      title: "IMG_3987",
      flickr: true,
    });
    expect(originalSource('<a href="https://www.flickr.com/photos/x/1/">https://www.flickr.com/photos/x/1/</a>')?.title).toBeNull();
    expect(originalSource('<span>Own work</span> <a href="//commons.wikimedia.org/wiki/File:X.jpg">x</a>')).toBeNull();
    expect(originalSource('Transferred from <a href="https://en.wikipedia.org">en.wikipedia</a>')).toBeNull();
  });
  it("never takes a Wikimedia tool, a plain-http link or a generic link text as the source (review round 2, S4)", () => {
    // LMT, QCOM and RCL: transferred with CommonsHelper.
    expect(
      originalSource(
        'Transferred from <a href="https://en.wikipedia.org">en.wikipedia</a> to Commons by <a href="//commons.wikimedia.org/wiki/User:Kelly">User:Kelly</a> using <a rel="nofollow" class="external text" href="http://tools.wikimedia.de/~magnus/commonshelper.php">CommonsHelper</a>.',
      ),
    ).toBeNull();
    expect(originalSource('<a href="https://x.toolforge.org/a">tool</a>')).toBeNull();
    expect(originalSource('<a href="http://www.example.com/photo">A photo</a>')).toBeNull();
    // NKE: the DPLA "File URL" link is a source, but its text is not a title.
    expect(originalSource('<a rel="nofollow" class="external text" href="https://nwdh-omeka.s3.us-west-2.amazonaws.com/original/d1.jpg">File URL</a>')).toEqual({
      url: "https://nwdh-omeka.s3.us-west-2.amazonaws.com/original/d1.jpg",
      title: null,
      flickr: false,
    });
  });
});
